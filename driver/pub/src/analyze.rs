//! Joins every compiled unit's facts into one graph of definitions, decides what reaches
//! each from the workspaces' entry points, and which `pub` tokens no other crate needs.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::cargo::{Package, Workspace};
use crate::collect::is_pub;
use crate::facts::{Hash, Unit};

/// How far a definition is reached, from least to most.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
	/// Nothing the workspaces run reaches it.
	Unreached,
	/// Only tests reach it.
	TestOnly,
	/// Code that runs reaches it, all from its own crate.
	CrateOnly,
	/// Another crate names it.
	CrossCrate,
}

/// A place in source: an absolute file and its bytes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Place {
	pub file: PathBuf,
	pub lo: u32,
	pub hi: u32,
}

/// A definition, joined across every compilation of its crate.
pub struct Def {
	pub crate_name: String,
	pub kind: String,
	pub path: String,
	pub span: Option<Place>,
	/// Whether another crate can name it.
	pub exported: bool,
	/// Part of a library's API that crates or tools outside the audit may use.
	pub external: bool,
	pub symbol: bool,
	pub category: Category,
	/// The first other crate's definition that names it, and where.
	pub cross: Option<Evidence>,
	/// The first definition in its own crate that names it, and where.
	pub local: Option<Evidence>,
}

/// A definition that names another, and where.
#[derive(Clone)]
pub struct Evidence {
	pub from: usize,
	pub site: Option<Place>,
}

/// A `pub` written in source, and every definition it makes public.
pub struct Token {
	pub place: Place,
	pub defs: Vec<usize>,
	pub verdict: Verdict,
}

/// Whether a token can narrow to `pub(crate)`, as far as the graph sees.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
	/// No other crate names what it makes public.
	Narrow,
	/// Another crate in the audit names one of its definitions.
	CrossCrate,
	/// One of its definitions is the API of a library crates or tools outside the audit
	/// may use, or it is in such a macro's definition, which copies it where it expands.
	External,
	/// One of its definitions is a symbol code outside Rust links to.
	Symbol,
	/// It is not in a local package's source, such as a dependency's macro or generated
	/// code.
	Generated,
	/// Its file is one a crate embeds as bytes, which narrowing would change.
	Embedded,
}

/// The joined graph.
pub struct Audit {
	pub defs: Vec<Def>,
	pub tokens: Vec<Token>,
	/// Each compilation's definitions, by hash.
	pub hashes: HashMap<Hash, usize>,
}

impl Audit {
	pub fn new(workspaces: &[Workspace], units: &[Unit]) -> Result<Self, String> {
		let packages = crate::cargo::packages(workspaces);
		let consumed = consumed(&packages);
		let mut defs: Vec<Def> = Vec::new();
		let mut keys: HashMap<(String, Option<Place>, String, String), usize> = HashMap::new();
		let mut hashes = HashMap::new();
		let mut tokens: BTreeMap<Place, Vec<usize>> = BTreeMap::new();
		let place = |unit: &Unit, span: crate::facts::Span| Place {
			file: unit.files[span.file as usize].clone(),
			lo: span.lo,
			hi: span.hi,
		};
		for unit in units {
			let package = packages.get(unit.package.as_path());
			// A published library's API is what it exports. An unpublished one the audit
			// does not run is used outside cargo's view, as a translator's fixture is by
			// the translator, which may read any of its source.
			let published = package.is_some_and(|p| p.publishable);
			let foreign = unit.lib && !published && !consumed.contains(unit.package.as_path());
			for def in &unit.defs {
				let span = def.span.map(|s| place(unit, s));
				let key = (
					unit.crate_name.clone(),
					span.clone(),
					def.kind.clone(),
					def.path.clone(),
				);
				let index = *keys.entry(key).or_insert_with(|| {
					defs.push(Def {
						crate_name: unit.crate_name.clone(),
						kind: def.kind.clone(),
						path: def.path.clone(),
						span,
						exported: false,
						external: false,
						symbol: false,
						category: Category::Unreached,
						cross: None,
						local: None,
					});
					defs.len() - 1
				});
				let joined = &mut defs[index];
				joined.exported |= unit.lib && def.exported;
				joined.external |= foreign || unit.lib && def.exported && published;
				joined.symbol |= def.symbol;
				hashes.insert(def.hash, index);
				if let Some(token) = def.token {
					let token = tokens.entry(place(unit, token)).or_default();
					if !token.contains(&index) {
						token.push(index);
					}
				}
			}
		}

		// What an import supplies from outside the audit, such as a dependency's type it
		// re-exports, has no definition here: give it a node, which the code naming it
		// reaches and which revives the import. A trait impl still requires only what
		// the audit defines.
		let collected = defs.len();
		for i in units.iter().flat_map(|u| &u.imports) {
			hashes.entry(i.supplied).or_insert_with(|| {
				defs.push(Def {
					crate_name: String::new(),
					kind: "Extern".into(),
					path: String::new(),
					span: None,
					exported: false,
					external: false,
					symbol: false,
					category: Category::Unreached,
					cross: None,
					local: None,
				});
				defs.len() - 1
			});
		}

		// Production and test edges, with who names what across crates.
		let n = defs.len();
		let mut edges = [vec![Vec::new(); n], vec![Vec::new(); n]];
		let mut roots = [Vec::new(), Vec::new()];
		let mut impls = [Vec::new(), Vec::new()];
		// What each definition's interface names, and the members of each type or module
		// that are its API too.
		let mut interface = vec![Vec::new(); n];
		let mut members = vec![Vec::new(); n];
		for unit in units {
			let test = usize::from(unit.test);
			for def in &unit.defs {
				let index = hashes[&def.hash];
				if let Some(&parent) = def.parent.and_then(|p| hashes.get(&p)) {
					// An import is live once what it imports is, which says nothing of the
					// function or module it is in.
					if def.kind != "Use" {
						edges[test][index].push(parent);
					}
					// A type's fields and variants, and a trait's items, are its API; a
					// module's, only what its crate exports from it.
					let member = match defs[parent].kind.as_str() {
						"Struct" | "Enum" | "Union" | "Variant" | "Trait" => true,
						"Mod" => def.exported,
						_ => false,
					};
					if member {
						members[parent].push(index);
					}
				}
				if let Some(&owner) = def.member_of.and_then(|p| hashes.get(&p)) {
					members[owner].push(index);
				}
			}
			for r in &unit.refs {
				let (Some(&from), Some(&to)) = (hashes.get(&r.from), hashes.get(&r.to)) else {
					continue;
				};
				edges[test][from].push(to);
				if r.interface {
					interface[from].push(to);
				}
				let evidence = Evidence {
					from,
					site: r.site.map(|s| place(unit, s)),
				};
				// A hash's first half is the crate that defines it.
				if r.to[0] != unit.crate_id {
					defs[to].cross.get_or_insert(evidence);
				} else if from != to {
					defs[to].local.get_or_insert(evidence);
				}
			}
			// A call names what an import supplies, not the import: the import, and the
			// re-exports a glob supplied it through, are live once it is, and they are
			// named as it is.
			for i in &unit.imports {
				let (import, supplied) = (hashes[&i.import], hashes[&i.supplied]);
				edges[test][supplied].push(import);
				for hash in std::iter::once(&i.supplied).chain(&i.through) {
					let Some(&to) = hashes.get(hash) else { continue };
					if to != supplied {
						edges[test][supplied].push(to);
					}
					let evidence = Evidence {
						from: import,
						site: None,
					};
					if hash[0] != unit.crate_id {
						defs[to].cross.get_or_insert(evidence);
					} else if import != to {
						defs[to].local.get_or_insert(evidence);
					}
				}
			}
			roots[test].extend(unit.roots.iter().filter_map(|r| hashes.get(r)));
			for i in &unit.impls {
				let requires: Vec<usize> = i
					.requires
					.iter()
					.filter_map(|r| hashes.get(r).copied().filter(|&d| d < collected))
					.collect();
				impls[test].push((hashes[&i.hash], requires));
			}
		}
		// What an external library's API exposes, such as a type of another crate it
		// re-exports or returns, is external too, with the type's own API.
		let external: Vec<usize> = (0..n).filter(|&d| defs[d].external).collect();
		for (d, reached) in reach(&[&interface, &members], &external, &[]).into_iter().enumerate() {
			defs[d].external |= reached;
		}
		let production = reach(&[&edges[0]], &roots[0], &impls[0]);
		let all_impls: Vec<_> = impls.concat();
		let all = reach(
			&[&edges[0], &edges[1]],
			&[roots[0].clone(), roots[1].clone()].concat(),
			&all_impls,
		);
		for (i, def) in defs.iter_mut().enumerate() {
			def.category = match (all[i], production[i], def.cross.is_some()) {
				(false, ..) => Category::Unreached,
				(true, false, _) => Category::TestOnly,
				(true, true, true) => Category::CrossCrate,
				(true, true, false) => Category::CrateOnly,
			};
		}

		let mut targets: Vec<&Path> = workspaces.iter().map(|w| w.target.as_path()).collect();
		targets.sort();
		// A macro outside crates may call copies its tokens there.
		let macros: Vec<&Place> = defs
			.iter()
			.filter(|d| d.external && d.kind.starts_with("Macro"))
			.filter_map(|d| d.span.as_ref())
			.collect();
		let embedded: BTreeSet<&Path> = units.iter().flat_map(|u| &u.embedded).map(PathBuf::as_path).collect();
		let tokens = tokens
			.into_iter()
			.map(|(place, token_defs)| {
				let mut verdict = verdict(&place, &token_defs, &defs, &packages, &targets)?;
				if verdict == Verdict::Narrow && embedded.contains(place.file.as_path()) {
					verdict = Verdict::Embedded;
				}
				if verdict == Verdict::Narrow
					&& macros
						.iter()
						.any(|m| m.file == place.file && m.lo <= place.lo && place.hi <= m.hi)
				{
					verdict = Verdict::External;
				}
				Ok(Token {
					place,
					defs: token_defs,
					verdict,
				})
			})
			.collect::<Result<_, String>>()?;
		Ok(Self { defs, tokens, hashes })
	}
}

/// The packages the audit consumes: those with a binary, which run, and the unpublished
/// workspace members such a package builds on.
fn consumed<'a>(packages: &BTreeMap<&'a Path, Package>) -> BTreeSet<&'a Path> {
	let mut consumed: BTreeSet<&Path> = packages
		.iter()
		.filter(|(_, p)| p.bin && p.member)
		.map(|(d, _)| *d)
		.collect();
	loop {
		let more: Vec<&Path> = packages
			.iter()
			.filter(|(dir, p)| {
				!consumed.contains(*dir) && p.member && p.dependents.iter().any(|d| consumed.contains(d.as_path()))
			})
			.map(|(d, _)| *d)
			.collect();
		if more.is_empty() {
			return consumed;
		}
		consumed.extend(more);
	}
}

fn verdict(
	place: &Place,
	token_defs: &[usize],
	defs: &[Def],
	packages: &BTreeMap<&Path, Package>,
	targets: &[&Path],
) -> Result<Verdict, String> {
	let local =
		packages.keys().any(|dir| place.file.starts_with(dir)) && !targets.iter().any(|t| place.file.starts_with(t));
	if !local {
		return Ok(Verdict::Generated);
	}
	let text = std::fs::read(&place.file).map_err(|err| format!("reading {}: {err}", place.file.display()))?;
	if !is_pub(&text, place.lo as usize) {
		return Err(format!(
			"{}: byte {} is not the `pub` rustc read there; was it edited during the audit?",
			place.file.display(),
			place.lo
		));
	}
	let defs = token_defs.iter().map(|&d| &defs[d]);
	Ok(if defs.clone().any(|d| d.cross.is_some()) {
		Verdict::CrossCrate
	} else if defs.clone().any(|d| d.external) {
		Verdict::External
	} else if defs.clone().any(|d| d.symbol) {
		Verdict::Symbol
	} else {
		Verdict::Narrow
	})
}

/// Which definitions `roots` reach over `edges`, with a trait impl reached once all it
/// requires is.
fn reach(edges: &[&Vec<Vec<usize>>], roots: &[usize], impls: &[(usize, Vec<usize>)]) -> Vec<bool> {
	let n = edges[0].len();
	let mut live = vec![false; n];
	let mut queue: VecDeque<usize> = VecDeque::new();
	for &r in roots {
		if !live[r] {
			live[r] = true;
			queue.push_back(r);
		}
	}
	loop {
		while let Some(d) = queue.pop_front() {
			for list in edges {
				for &next in &list[d] {
					if !live[next] {
						live[next] = true;
						queue.push_back(next);
					}
				}
			}
		}
		for (i, requires) in impls {
			if !live[*i] && requires.iter().all(|&r| live[r]) {
				live[*i] = true;
				queue.push_back(*i);
			}
		}
		if queue.is_empty() {
			return live;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::reach;

	#[test]
	fn impls_activate_once_trait_and_type_are_live() {
		// 0 main -> 1 trait, 2 type; 3 impl of 1 for 2 -> 4 method -> 5 helper; 6 dead
		// cycle with 7; 8 impl requiring dead 6.
		let mut edges = vec![Vec::new(); 9];
		edges[0] = vec![1, 2];
		edges[3] = vec![4];
		edges[4] = vec![5];
		edges[6] = vec![7];
		edges[7] = vec![6];
		let live = reach(&[&edges], &[0], &[(3, vec![1, 2]), (8, vec![6])]);
		assert_eq!(live, [true, true, true, true, true, true, false, false, false]);
	}
}
