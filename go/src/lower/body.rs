//! One function instance's MIR as a Go function: each local a variable, each block a
//! label, and each terminator a `goto`, a `return`, or a panic, as the MIR has them. A
//! panic is Go's, with native Rust's message, and runs no cleanup, as `panic = "abort"`
//! doesn't: the MIR's cleanup blocks are left out.

use std::collections::HashSet;
use std::fmt::Write;

use rustc_hir::def::DefKind;
use rustc_middle::mir::{
	self, AggregateKind, AssertKind, BasicBlock, BinOp, CastKind, Local, Operand, Place, PlaceElem, Rvalue,
	StatementKind, TerminatorKind, UnOp,
};
use rustc_middle::ty::adjustment::PointerCoercion;
use rustc_middle::ty::util::IntTypeExt;
use rustc_middle::ty::{self, EarlyBinder, Instance, InstanceKind, Ty, TyCtxt};
use rustc_span::Span;

use super::Translator;
use super::library::{self, Mapped};
use super::names;
use super::ty::variant_field;

/// Translate one function instance, or `None` after reporting what it refuses.
pub fn function<'tcx>(t: &mut Translator<'tcx>, instance: Instance<'tcx>, reached: Span) -> Option<String> {
	let tcx = t.tcx;
	let mir = instance_mir(t, instance, reached)?;
	let name = t.funcs[&instance].clone();
	let mut fx = Fx {
		t,
		tcx,
		mir: &mir,
		reached,
		local: instance.def_id().is_local(),
		used: HashSet::new(),
		mentioned: HashSet::new(),
		targets: HashSet::new(),
		succ: Vec::new(),
		sites: Vec::new(),
		ok: true,
	};
	let text = fx.function(&name, instance);
	fx.ok.then_some(text)
}

/// The MIR of `instance`, with its generic arguments in place.
fn instance_mir<'tcx>(t: &mut Translator<'tcx>, instance: Instance<'tcx>, reached: Span) -> Option<mir::Body<'tcx>> {
	let tcx = t.tcx;
	let def_id = instance.def_id();
	let body = match instance.def {
		InstanceKind::Item(_) if !tcx.is_mir_available(def_id) => {
			t.err(
				reached,
				format!("calls `{}`, which has no MIR and no binding", tcx.def_path_str(def_id)),
			);
			return None;
		}
		InstanceKind::Item(_) if tcx.def_kind(def_id) == DefKind::Const => tcx.mir_for_ctfe(def_id).clone(),
		_ => tcx.instance_mir(instance.def).clone(),
	};
	match instance.try_instantiate_mir_and_normalize_erasing_regions(tcx, t.env(), EarlyBinder::bind(tcx, body)) {
		Ok(body) => Some(body),
		Err(_) => {
			t.err(reached, format!("`{instance}` does not normalize"));
			None
		}
	}
}

pub struct Fx<'a, 'tcx> {
	pub t: &'a mut Translator<'tcx>,
	pub tcx: TyCtxt<'tcx>,
	mir: &'a mir::Body<'tcx>,
	/// What first reached this function, which a refusal in another crate's points at too.
	reached: Span,
	/// Whether this is the translated crate's own function.
	local: bool,
	/// Locals the body reads, which Go requires of every variable.
	used: HashSet<Local>,
	/// Locals the body names at all, which are the ones it declares.
	mentioned: HashSet<Local>,
	/// Where each `tracing` event in the body is invoked, which translation leaves out.
	sites: Vec<Span>,
	/// Blocks some `goto` jumps to, which Go requires of every label.
	targets: HashSet<BasicBlock>,
	/// The blocks the block being translated goes to.
	succ: Vec<BasicBlock>,
	ok: bool,
}

/// A place as Go reads and writes it, and its Rust type.
pub struct GoPlace<'tcx> {
	pub expr: String,
	pub ty: Ty<'tcx>,
}

impl<'a, 'tcx> Fx<'a, 'tcx> {
	/// Report a construct translation refuses, here and where it was reached from.
	pub fn err(&mut self, span: Span, msg: impl Into<String>) -> String {
		let msg = format!("{} is not supported", msg.into());
		self.ok = false;
		self.t.failed = true;
		if !self.t.reported.insert((span, msg.clone())) {
			return "struct{}{}".into();
		}
		let msg = format!("rust2-go: {msg}");
		let mut diag = self.tcx.dcx().struct_span_err(span, msg);
		if !self.local {
			diag = diag.with_span_note(self.reached, "reached from here");
		}
		diag.emit();
		"struct{}{}".into()
	}

	pub fn ty(&mut self, ty: Ty<'tcx>, span: Span) -> String {
		self.t.ty(ty, span)
	}

	fn local_name(local: Local) -> String {
		format!("_{}", local.as_usize())
	}

	fn function(&mut self, name: &str, instance: Instance<'tcx>) -> String {
		let mir = self.mir;
		let span = mir.span;
		let mut params = Vec::new();
		let mut prologue = String::new();
		for arg in mir.args_iter() {
			let ty = mir.local_decls[arg].ty;
			let name = Self::local_name(arg);
			if Some(arg) == mir.spread_arg {
				// rust-call's tuple arrives as its fields, as every caller spreads it.
				let ty::Tuple(tys) = ty.kind() else {
					self.err(span, "a spread argument that is not a tuple");
					continue;
				};
				let mut fields = Vec::new();
				for (i, field) in tys.iter().enumerate() {
					let go = self.ty(field, span);
					params.push(format!("{name}_{i} {go}"));
					fields.push(format!("{name}_{i}"));
				}
				let go = self.ty(ty, span);
				let value = match tys.is_empty() {
					true => "struct{}{}".to_string(),
					false => format!("{go}{{{}}}", fields.join(", ")),
				};
				writeln!(prologue, "\tvar {name} {go} = {value}").unwrap();
				self.used.insert(arg);
			} else {
				let go = self.ty(ty, span);
				params.push(format!("{name} {go}"));
			}
		}
		let ret_ty = mir.return_ty();
		let ret = match Translator::no_value(ret_ty) {
			true => String::new(),
			false => format!(" {}", self.ty(ret_ty, span)),
		};
		self.sites = self.tracing_sites();
		// Only the blocks the translation goes to, from the first: no cleanup, and no block
		// of a left-out event.
		let mut blocks = std::collections::BTreeMap::new();
		let mut stack = vec![mir::START_BLOCK];
		while let Some(bb) = stack.pop() {
			if blocks.contains_key(&bb) {
				continue;
			}
			let data = &mir.basic_blocks[bb];
			let mut text = String::new();
			let before = self.succ.len();
			for stmt in &data.statements {
				if self.in_site(stmt.source_info.span).is_none() {
					self.statement(stmt, &mut text);
				}
			}
			let term = data.terminator();
			match self.in_site(term.source_info.span) {
				Some(site) => match self.site_exit(bb, site) {
					Some(exit) => self.goto(exit, &mut text),
					None => {
						self.err(site, "a `tracing` event whose expansion has no single exit");
					}
				},
				None => self.terminator(term, &mut text),
			}
			blocks.insert(bb, text);
			stack.extend(self.succ.drain(before..));
		}
		let mut decls = String::new();
		for local in mir.local_decls.indices().skip(1 + mir.arg_count) {
			self.declare(local, &mut decls);
		}
		self.declare(mir::RETURN_PLACE, &mut decls);
		for arg in mir.args_iter() {
			if !self.used.contains(&arg) {
				writeln!(decls, "\t_ = {}", Self::local_name(arg)).unwrap();
			}
		}
		let mut out = format!(
			"// {name} is `{instance}`.\nfunc {name}({}){ret} {{\n",
			params.join(", ")
		);
		out.push_str(&prologue);
		out.push_str(&decls);
		for (bb, text) in blocks {
			if self.targets.contains(&bb) {
				writeln!(out, "bb{}:", bb.as_usize()).unwrap();
			}
			out.push_str(&text);
		}
		out.push_str("}\n");
		out
	}

	/// Where each `tracing` event the body holds is invoked: the outermost of its macros,
	/// which expand to one another. Translated code installs no subscriber, and without one
	/// an event evaluates none of its fields and does nothing, so leaving it out is exact,
	/// as it is in rust2-ts.
	fn tracing_sites(&self) -> Vec<Span> {
		let mut sites = Vec::new();
		let spans = self.mir.basic_blocks.iter().flat_map(|data| {
			data.statements
				.iter()
				.map(|s| s.source_info.span)
				.chain(std::iter::once(data.terminator().source_info.span))
		});
		for mut span in spans {
			let mut site = None;
			while span.from_expansion() {
				let data = span.ctxt().outer_expn_data();
				if let Some(def_id) = data.macro_def_id
					&& self.tcx.crate_name(def_id.krate).as_str() == "tracing"
					&& matches!(
						self.tcx.item_name(def_id).as_str(),
						"event" | "trace" | "debug" | "info" | "warn" | "error"
					) {
					site = Some(data.call_site);
				}
				span = data.call_site;
			}
			if let Some(site) = site
				&& !sites.contains(&site)
			{
				sites.push(site);
			}
		}
		sites
	}

	/// The `tracing` event whose invocation holds `span`, its arguments included.
	fn in_site(&self, span: Span) -> Option<Span> {
		let root = span.source_callsite();
		self.sites.iter().copied().find(|site| site.contains(root))
	}

	/// The one block an event's blocks, from `bb`'s on, go on to.
	fn site_exit(&self, bb: BasicBlock, site: Span) -> Option<BasicBlock> {
		let blocks = &self.mir.basic_blocks;
		let mut seen = HashSet::from([bb]);
		let mut stack = vec![bb];
		let mut exits = HashSet::new();
		while let Some(next) = stack.pop() {
			for succ in blocks[next].terminator().successors() {
				if blocks[succ].is_cleanup || !seen.insert(succ) {
					continue;
				}
				match self.in_site(blocks[succ].terminator().source_info.span) == Some(site) {
					true => stack.push(succ),
					false => {
						exits.insert(succ);
					}
				}
			}
		}
		let mut exits = exits.into_iter();
		match (exits.next(), exits.next()) {
			(Some(exit), None) => Some(exit),
			_ => None,
		}
	}

	/// Declare a local the body names, and use it if nothing reads it, as Go requires.
	fn declare(&mut self, local: Local, out: &mut String) {
		let ty = self.mir.local_decls[local].ty;
		if ty.is_never() || !self.mentioned.contains(&local) {
			return;
		}
		let go = self.ty(ty, self.mir.local_decls[local].source_info.span);
		let name = Self::local_name(local);
		writeln!(out, "\tvar {name} {go}").unwrap();
		if !self.used.contains(&local) {
			writeln!(out, "\t_ = {name}").unwrap();
		}
	}

	fn statement(&mut self, stmt: &mir::Statement<'tcx>, out: &mut String) {
		let span = stmt.source_info.span;
		match &stmt.kind {
			StatementKind::Assign(assign) => {
				let (place, rvalue) = &**assign;
				let ty = place.ty(self.mir, self.tcx).ty;
				let value = self.rvalue(rvalue, ty, span);
				let dest = self.dest(*place, span);
				writeln!(out, "\t{dest} = {value}").unwrap();
			}
			StatementKind::SetDiscriminant { place, variant_index } => {
				let place = self.place(**place, span);
				let ty::Adt(adt, _) = place.ty.kind() else {
					self.err(span, "a discriminant of a type that is no enum");
					return;
				};
				let discr = self.discr(*adt, *variant_index, span);
				writeln!(out, "\t{}.tag = {discr}", place.expr).unwrap();
			}
			StatementKind::StorageLive(_)
			| StatementKind::StorageDead(_)
			| StatementKind::Nop
			| StatementKind::FakeRead(_)
			| StatementKind::AscribeUserType(..)
			| StatementKind::Coverage(_)
			| StatementKind::PlaceMention(_)
			| StatementKind::ConstEvalCounter
			| StatementKind::BackwardIncompatibleDropHint { .. } => {}
			StatementKind::Intrinsic(intrinsic) => match &**intrinsic {
				mir::NonDivergingIntrinsic::Assume(_) => {}
				mir::NonDivergingIntrinsic::CopyNonOverlapping(_) => {
					self.err(span, "`copy_nonoverlapping`");
				}
			},
		}
	}

	/// A place written to: its local is no read of it.
	fn dest(&mut self, place: Place<'tcx>, span: Span) -> String {
		let used = self.used.contains(&place.local);
		let expr = self.place(place, span).expr;
		if place.projection.is_empty() && !used {
			self.used.remove(&place.local);
		}
		expr
	}

	/// A place's Go expression.
	pub fn place(&mut self, place: Place<'tcx>, span: Span) -> GoPlace<'tcx> {
		self.used.insert(place.local);
		self.mentioned.insert(place.local);
		let mut expr = Self::local_name(place.local);
		let mut pty = mir::PlaceTy::from_ty(self.mir.local_decls[place.local].ty);
		for elem in place.projection.iter() {
			let base = pty.ty;
			match elem {
				PlaceElem::Deref => match base.kind() {
					ty::Ref(_, inner, _) | ty::RawPtr(inner, _) => {
						// A slice, a `str`, or a trait object is its fat pointer.
						if !matches!(inner.kind(), ty::Slice(_) | ty::Str | ty::Dynamic(..)) {
							expr = format!("(*{expr})");
						}
					}
					_ => {
						return GoPlace {
							expr: self.err(span, format!("a dereference of `{base}`")),
							ty: base,
						};
					}
				},
				PlaceElem::Field(field, _) => match base.kind() {
					// What a box and `MaybeUninit` hold is where they are.
					ty::Adt(adt, _)
						if matches!(
							library::mapped(self.tcx, adt.did()),
							Some(Mapped::Box | Mapped::Transparent)
						) && field.as_usize() == usize::from(adt.is_union()) => {}
					ty::Adt(adt, _) if library::mapped(self.tcx, adt.did()).is_some_and(|m| !m.is_runtime_layout()) => {
						return GoPlace {
							expr: self.err(
								span,
								format!("a field of `{base}`, which Go holds as a type of its own"),
							),
							ty: base,
						};
					}
					ty::Adt(adt, _) => {
						let variant = pty.variant_index.unwrap_or(rustc_abi::FIRST_VARIANT);
						let def = adt.variant(variant);
						let name = names::field(def.fields[field].name.as_str(), field.as_usize());
						expr = match adt.is_enum() {
							true => format!("{expr}.{}.{name}", variant_field(def.name.as_str())),
							false => format!("{expr}.{name}"),
						};
					}
					ty::Tuple(_) | ty::Closure(..) => expr = format!("{expr}.F{}", field.as_usize()),
					_ => {
						return GoPlace {
							expr: self.err(span, format!("a field of `{base}`")),
							ty: base,
						};
					}
				},
				PlaceElem::Downcast(..)
				| PlaceElem::OpaqueCast(_)
				| PlaceElem::UnwrapUnsafeBinder(_)
				| PlaceElem::PhantomDeref => {}
				PlaceElem::Index(index) => {
					self.used.insert(index);
					self.mentioned.insert(index);
					expr = format!("{expr}[{}]", Self::local_name(index));
				}
				PlaceElem::ConstantIndex {
					offset,
					from_end: false,
					..
				} => expr = format!("{expr}[{offset}]"),
				PlaceElem::ConstantIndex {
					offset, from_end: true, ..
				} => expr = format!("{expr}[len({expr})-{offset}]"),
				PlaceElem::Subslice {
					from,
					to,
					from_end: true,
				} if matches!(base.kind(), ty::Slice(_)) => expr = format!("{expr}[{from}:len({expr})-{to}]"),
				// Part of an array, as an array: the place a pointer to it converted from a slice
				// of it is, which aliases the array as Rust's does.
				PlaceElem::Subslice { from, to, from_end } => {
					let ty::Array(elem, len) = base.kind() else {
						return GoPlace {
							expr: self.err(span, format!("a subslice of `{base}`")),
							ty: base,
						};
					};
					let len = len.try_to_target_usize(self.tcx).unwrap_or(0);
					let end = if from_end { len - to } else { to };
					let elem = self.ty(*elem, span);
					expr = format!("(*(*[{}]{elem})({expr}[{from}:{end}]))", end - from);
				}
			}
			pty = pty.projection_ty(self.tcx, elem);
		}
		GoPlace { expr, ty: pty.ty }
	}

	pub fn operand(&mut self, op: &Operand<'tcx>, span: Span) -> String {
		match op {
			Operand::Copy(place) | Operand::Move(place) => self.place(*place, span).expr,
			Operand::Constant(c) => self.constant(c, span),
			Operand::RuntimeChecks(_) => "false".into(),
		}
	}

	fn operand_ty(&self, op: &Operand<'tcx>) -> Ty<'tcx> {
		op.ty(self.mir, self.tcx)
	}

	/// The value of an enum's tag for `variant`, as a Go literal of its type.
	pub fn discr(&mut self, adt: ty::AdtDef<'tcx>, variant: rustc_abi::VariantIdx, span: Span) -> String {
		let tcx = self.tcx;
		let discr_ty = adt.repr().discr_type().to_ty(tcx);
		let value = adt.discriminant_for_variant(tcx, variant).val;
		self.int_lit(discr_ty, value, span)
	}

	/// An integer of type `ty` from its bits, as a typed Go literal.
	pub fn int_lit(&mut self, ty: Ty<'tcx>, bits: u128, span: Span) -> String {
		let go = self.ty(ty, span);
		let size = int_bits(self.tcx, ty);
		let text = match ty.kind() {
			ty::Int(_) => {
				let shift = 128 - size;
				(((bits << shift) as i128) >> shift).to_string()
			}
			ty::Bool => return (bits != 0).to_string(),
			_ => (bits & (u128::MAX >> (128 - size))).to_string(),
		};
		format!("{go}({text})")
	}

	fn rvalue(&mut self, rvalue: &Rvalue<'tcx>, dest_ty: Ty<'tcx>, span: Span) -> String {
		let tcx = self.tcx;
		match rvalue {
			Rvalue::Use(op, _) => self.operand(op, span),
			Rvalue::CopyForDeref(place) => self.place(*place, span).expr,
			Rvalue::Repeat(op, _) => {
				let v = self.operand(op, span);
				let go = self.ty(dest_ty, span);
				format!("func() (a {go}) {{ for i := range a {{ a[i] = {v} }}; return }}()")
			}
			Rvalue::Ref(_, _, place) | Rvalue::RawPtr(_, place) => {
				let place = self.place(*place, span);
				match place.ty.kind() {
					ty::Slice(_) | ty::Str | ty::Dynamic(..) => place.expr,
					_ => format!("&{}", place.expr),
				}
			}
			Rvalue::Discriminant(place) => {
				let place = self.place(*place, span);
				match place.ty.kind() {
					ty::Adt(adt, _) if adt.is_enum() => {
						let tag_ty = adt.repr().discr_type().to_ty(tcx);
						let go = self.ty(dest_ty, span);
						match tag_ty == dest_ty {
							true => format!("{}.tag", place.expr),
							false => format!("{go}({}.tag)", place.expr),
						}
					}
					_ => format!("{}(0)", self.ty(dest_ty, span)),
				}
			}
			Rvalue::Cast(kind, op, to) => self.cast(*kind, op, *to, span),
			Rvalue::BinaryOp(op, ops) => self.binary(*op, &ops.0, &ops.1, dest_ty, span),
			Rvalue::UnaryOp(op, operand) => {
				let v = self.operand(operand, span);
				let from = self.operand_ty(operand);
				match op {
					UnOp::Not if from.is_bool() => format!("!{v}"),
					UnOp::Not => format!("^{v}"),
					UnOp::Neg => format!("-{v}"),
					UnOp::PtrMetadata => match from.builtin_deref(true).map(|t| t.kind()) {
						Some(ty::Slice(_) | ty::Str) => format!("uint(len({v}))"),
						_ if dest_ty.is_unit() => "struct{}{}".into(),
						_ => self.err(span, format!("the metadata of `{from}`")),
					},
				}
			}
			Rvalue::Aggregate(kind, ops) => self.aggregate(kind, ops, dest_ty, span),
			Rvalue::ThreadLocalRef(_) => self.err(span, "a thread-local"),
			Rvalue::Reborrow(..) => self.err(span, "reborrowing by a `Reborrow` impl"),
			Rvalue::WrapUnsafeBinder(op, _) => self.operand(op, span),
		}
	}

	fn aggregate(
		&mut self,
		kind: &AggregateKind<'tcx>,
		ops: &rustc_index::IndexSlice<rustc_abi::FieldIdx, Operand<'tcx>>,
		dest_ty: Ty<'tcx>,
		span: Span,
	) -> String {
		let values: Vec<String> = ops.iter().map(|op| self.operand(op, span)).collect();
		let go = self.ty(dest_ty, span);
		match kind {
			AggregateKind::Array(_) | AggregateKind::Closure(..) => format!("{go}{{{}}}", values.join(", ")),
			AggregateKind::Tuple if values.is_empty() => "struct{}{}".into(),
			AggregateKind::Tuple => format!("{go}{{{}}}", values.join(", ")),
			AggregateKind::Adt(did, variant, args, _, active) => {
				let adt = self.tcx.adt_def(*did);
				if library::mapped(self.tcx, *did) == Some(Mapped::Transparent) {
					// The value it holds, or Go's zero value for `MaybeUninit::uninit()`.
					return match (adt.is_union(), active) {
						(true, Some(field)) if field.as_usize() == 0 => format!("*new({go})"),
						_ => values[0].clone(),
					};
				}
				if active.is_some() {
					return self.err(span, "a union");
				}
				self.adt_lit(adt, *variant, args, &values, &go, span)
			}
			_ => self.err(span, format!("the aggregate `{kind:?}`")),
		}
	}

	/// An ADT's value of `variant` from its fields' values, as a Go composite literal of
	/// its type `go`.
	pub fn adt_lit(
		&mut self,
		adt: ty::AdtDef<'tcx>,
		variant: rustc_abi::VariantIdx,
		args: ty::GenericArgsRef<'tcx>,
		values: &[String],
		go: &str,
		span: Span,
	) -> String {
		if let Some(mapped) = library::mapped(self.tcx, adt.did())
			&& !mapped.is_runtime_layout()
		{
			return match mapped {
				Mapped::Runtime("struct{}") => "struct{}{}".into(),
				_ => self.err(
					span,
					format!("making a `{}` from its fields", self.tcx.def_path_str(adt.did())),
				),
			};
		}
		let def = adt.variant(variant);
		let fields: Vec<String> = def
			.fields
			.iter()
			.enumerate()
			.zip(values)
			.map(|((i, f), v)| format!("{}: {v}", names::field(f.name.as_str(), i)))
			.collect();
		if !adt.is_enum() {
			return format!("{go}{{{}}}", fields.join(", "));
		}
		let discr = self.discr(adt, variant, span);
		if fields.is_empty() {
			return format!("{go}{{tag: {discr}}}");
		}
		let mut types = Vec::new();
		for (i, f) in def.fields.iter().enumerate() {
			let fty = f.ty(self.tcx, args).skip_norm_wip();
			let fty = self
				.tcx
				.normalize_erasing_regions(self.t.env(), ty::Unnormalized::new_wip(fty));
			let fgo = self.ty(fty, span);
			types.push(format!("{} {fgo}", names::field(f.name.as_str(), i)));
		}
		format!(
			"{go}{{tag: {discr}, {}: struct{{ {} }}{{{}}}}}",
			variant_field(def.name.as_str()),
			types.join("; "),
			values.join(", ")
		)
	}

	pub fn t_env(&self) -> ty::TypingEnv<'tcx> {
		self.t.env()
	}

	pub fn mir_decls(&self) -> &'a mir::LocalDecls<'tcx> {
		&self.mir.local_decls
	}

	fn cast(&mut self, kind: CastKind, op: &Operand<'tcx>, to: Ty<'tcx>, span: Span) -> String {
		let from = self.operand_ty(op);
		let v = self.operand(op, span);
		let go = self.ty(to, span);
		match kind {
			CastKind::IntToInt => match from.kind() {
				ty::Bool => format!("boolTo[{go}]({v})"),
				ty::Adt(adt, _) if adt.is_enum() => format!("{go}({v}.tag)"),
				_ => format!("{go}({v})"),
			},
			CastKind::PtrToPtr | CastKind::Transmute | CastKind::Subtype | CastKind::BoxDerefTransmute => {
				let from_go = self.ty(from, span);
				match from_go == go {
					true => v,
					false => self.err(span, format!("a cast from `{from}` to `{to}`")),
				}
			}
			CastKind::PointerCoercion(coercion, _) => match coercion {
				PointerCoercion::Unsize => {
					let inner = to.builtin_deref(true);
					match inner.map(|t| t.kind()) {
						Some(ty::Slice(_)) => format!("{v}[:]"),
						Some(ty::Dynamic(..)) => self.dyn_value(from, to, v, span),
						_ => self.err(span, format!("an unsizing of `{from}` to `{to}`")),
					}
				}
				PointerCoercion::ReifyFnPointer(_) => {
					let ty::FnDef(def_id, args) = *from.kind() else {
						return self.err(span, "a function pointer to a value that is no function");
					};
					match Instance::resolve_for_fn_ptr(self.tcx, self.t.env(), def_id, args.skip_binder()) {
						Some(instance) => self.t.func(instance, span),
						None => self.err(span, "a function pointer that does not resolve"),
					}
				}
				PointerCoercion::MutToConstPointer
				| PointerCoercion::ArrayToPointer
				| PointerCoercion::UnsafeFnPointer => v,
				_ => self.err(span, format!("the coercion `{coercion:?}`")),
			},
			_ => self.err(span, format!("the cast `{kind:?}` from `{from}` to `{to}`")),
		}
	}

	/// A trait object of `Display` or `Debug` made from a reference to a value of `from`'s
	/// pointee: the runtime's `fmtDyn`, calling the impl's `fmt`.
	fn dyn_value(&mut self, from: Ty<'tcx>, to: Ty<'tcx>, v: String, span: Span) -> String {
		let tcx = self.tcx;
		let (Some(pointee), Some(ty::Dynamic(preds, ..))) =
			(from.builtin_deref(true), to.builtin_deref(true).map(|t| t.kind()))
		else {
			return self.err(span, format!("a trait object made from `{from}`"));
		};
		let Some(trait_id) = preds.principal_def_id() else {
			return self.err(span, format!("a trait object made from `{from}`"));
		};
		let name = tcx.item_name(trait_id).to_string();
		match library::fmt_call(self, &name, pointee, &v, span) {
			Some(call) => format!("fmtDyn({call})"),
			None => "nil".into(),
		}
	}

	/// `fmt::Result`, which formatting functions return.
	pub fn fmt_result_ty(&self) -> Ty<'tcx> {
		let tcx = self.tcx;
		let result = tcx
			.get_diagnostic_item(rustc_span::sym::Result)
			.expect("std's `Result`");
		let error = self.fmt_error_ty().expect("`fmt::Error`");
		Ty::new_adt(
			tcx,
			tcx.adt_def(result),
			tcx.mk_args(&[tcx.types.unit.into(), error.into()]),
		)
	}

	fn fmt_error_ty(&self) -> Option<Ty<'tcx>> {
		let tcx = self.tcx;
		let core = tcx
			.crates(())
			.iter()
			.copied()
			.find(|&c| tcx.crate_name(c).as_str() == "core")?;
		let fmt = super::names::resolve_in(tcx, core, &["fmt", "Error"])?;
		Some(tcx.type_of(fmt).instantiate_identity().skip_norm_wip())
	}

	/// `&mut Formatter`, which formatting functions take.
	pub fn formatter_ref_ty(&self) -> Ty<'tcx> {
		let tcx = self.tcx;
		let core = tcx
			.crates(())
			.iter()
			.copied()
			.find(|&c| tcx.crate_name(c).as_str() == "core")
			.expect("core");
		let f = super::names::resolve_in(tcx, core, &["fmt", "Formatter"]).expect("`fmt::Formatter`");
		let ty = Ty::new_adt(tcx, tcx.adt_def(f), tcx.mk_args(&[tcx.lifetimes.re_erased.into()]));
		Ty::new_mut_ref(tcx, tcx.lifetimes.re_erased, ty)
	}

	fn binary(&mut self, op: BinOp, a: &Operand<'tcx>, b: &Operand<'tcx>, dest_ty: Ty<'tcx>, span: Span) -> String {
		let ty = self.operand_ty(a);
		let x = self.operand(a, span);
		let y = self.operand(b, span);
		let signed = ty.is_signed();
		let bits = match ty.is_integral() {
			true => int_bits(self.tcx, ty),
			false => 0,
		};
		if ty.is_bool() {
			return match op {
				BinOp::BitAnd => format!("({x} && {y})"),
				BinOp::BitOr => format!("({x} || {y})"),
				BinOp::BitXor | BinOp::Ne => format!("({x} != {y})"),
				BinOp::Eq => format!("({x} == {y})"),
				BinOp::Lt => format!("(!{x} && {y})"),
				BinOp::Gt => format!("({x} && !{y})"),
				BinOp::Le => format!("(!{x} || {y})"),
				BinOp::Ge => format!("({x} || !{y})"),
				_ => self.err(span, format!("`{op:?}` of `bool`")),
			};
		}
		if !(ty.is_integral() || ty.is_char() || ty.is_raw_ptr() || ty.is_ref()) {
			return self.err(span, format!("`{op:?}` of `{ty}`"));
		}
		let sign = if signed { "S" } else { "U" };
		match op {
			BinOp::Add | BinOp::AddUnchecked => format!("({x} + {y})"),
			BinOp::Sub | BinOp::SubUnchecked => format!("({x} - {y})"),
			BinOp::Mul | BinOp::MulUnchecked => format!("({x} * {y})"),
			BinOp::Div => format!("({x} / {y})"),
			BinOp::Rem => format!("({x} % {y})"),
			BinOp::BitAnd => format!("({x} & {y})"),
			BinOp::BitOr => format!("({x} | {y})"),
			BinOp::BitXor => format!("({x} ^ {y})"),
			// MIR's shifts take the amount modulo the width, which Go's do not.
			BinOp::Shl => format!("({x} << (uint({y}) & {}))", bits - 1),
			BinOp::Shr => format!("({x} >> (uint({y}) & {}))", bits - 1),
			BinOp::ShlUnchecked => format!("({x} << uint({y}))"),
			BinOp::ShrUnchecked => format!("({x} >> uint({y}))"),
			BinOp::Eq => format!("({x} == {y})"),
			BinOp::Ne => format!("({x} != {y})"),
			BinOp::Lt => format!("({x} < {y})"),
			BinOp::Le => format!("({x} <= {y})"),
			BinOp::Gt => format!("({x} > {y})"),
			BinOp::Ge => format!("({x} >= {y})"),
			BinOp::Cmp => format!("cmp3({x}, {y})"),
			BinOp::AddWithOverflow => format!("add{sign}({x}, {y})"),
			BinOp::SubWithOverflow => format!("sub{sign}({x}, {y})"),
			BinOp::MulWithOverflow => format!("mul{sign}({x}, {y})"),
			_ => {
				let _ = dest_ty;
				self.err(span, format!("`{op:?}` of `{ty}`"))
			}
		}
	}

	fn terminator(&mut self, term: &mir::Terminator<'tcx>, out: &mut String) {
		let span = term.source_info.span;
		match &term.kind {
			TerminatorKind::Goto { target, .. } => self.goto(*target, out),
			TerminatorKind::SwitchInt { discr, targets } => {
				let ty = self.operand_ty(discr);
				let v = self.operand(discr, span);
				if ty.is_bool() {
					let (zero, other) = (targets.target_for_value(0), targets.otherwise());
					writeln!(out, "\tif {v} {{").unwrap();
					let t = if targets.iter().any(|(val, _)| val == 1) {
						targets.target_for_value(1)
					} else {
						other
					};
					self.goto(t, out);
					writeln!(out, "\t}}").unwrap();
					self.goto(zero, out);
					return;
				}
				writeln!(out, "\tswitch {v} {{").unwrap();
				for (value, target) in targets.iter() {
					let lit = match ty.kind() {
						ty::Char => format!("{value}"),
						_ => self.int_lit(ty, value, span),
					};
					writeln!(out, "\tcase {lit}:").unwrap();
					self.goto(target, out);
				}
				writeln!(out, "\t}}").unwrap();
				self.goto(targets.otherwise(), out);
			}
			TerminatorKind::Return => match Translator::no_value(self.mir.return_ty()) {
				true => writeln!(out, "\treturn").unwrap(),
				false => {
					self.used.insert(mir::RETURN_PLACE);
					self.mentioned.insert(mir::RETURN_PLACE);
					writeln!(out, "\treturn _0").unwrap()
				}
			},
			TerminatorKind::Unreachable => writeln!(out, "\tpanic(\"unreachable\")").unwrap(),
			TerminatorKind::UnwindResume | TerminatorKind::UnwindTerminate(_) => {
				writeln!(out, "\tpanic(\"unwind\")").unwrap()
			}
			TerminatorKind::Drop { place, target, .. } => {
				let ty = place.ty(self.mir, self.tcx).ty;
				if let Some(why) = library::drop_refused(self.tcx, self.t.env(), ty) {
					self.err(span, format!("dropping `{ty}`, {why}"));
				}
				self.goto(*target, out);
			}
			TerminatorKind::Call {
				func,
				args,
				destination,
				target,
				fn_span,
				..
			} => {
				let args: Vec<&Operand<'tcx>> = args.iter().map(|a| &a.node).collect();
				let start = out.len();
				self.call(func, &args, *destination, *fn_span, out);
				match target {
					Some(target) => self.goto(*target, out),
					None if out[start..].trim_start().starts_with("panic(") => {}
					None => writeln!(out, "\tpanic(\"unreachable\")").unwrap(),
				}
			}
			TerminatorKind::Assert {
				cond,
				expected,
				msg,
				target,
				..
			} => {
				let c = self.operand(cond, span);
				let check = if *expected { format!("!{c}") } else { c };
				let msg = self.assert_message(msg, span);
				writeln!(out, "\tif {check} {{\n\t\tpanic({msg})\n\t}}").unwrap();
				self.goto(*target, out);
			}
			TerminatorKind::FalseEdge { real_target, .. } => self.goto(*real_target, out),
			TerminatorKind::FalseUnwind { real_target, .. } => self.goto(*real_target, out),
			kind => {
				self.err(span, format!("the terminator `{:?}`", kind.name()));
			}
		}
	}

	fn goto(&mut self, target: BasicBlock, out: &mut String) {
		self.targets.insert(target);
		self.succ.push(target);
		writeln!(out, "\tgoto bb{}", target.as_usize()).unwrap();
	}

	/// The panic of a failed assertion, with native Rust's message.
	fn assert_message(&mut self, msg: &AssertKind<Operand<'tcx>>, span: Span) -> String {
		let text = match msg {
			AssertKind::BoundsCheck { len, index } => {
				let len = self.operand(len, span);
				let index = self.operand(index, span);
				return format!("panicBounds({len}, {index})");
			}
			AssertKind::Overflow(op, ..) => match op {
				BinOp::Add => "attempt to add with overflow",
				BinOp::Sub => "attempt to subtract with overflow",
				BinOp::Mul => "attempt to multiply with overflow",
				BinOp::Div => "attempt to divide with overflow",
				BinOp::Rem => "attempt to calculate the remainder with overflow",
				BinOp::Shl => "attempt to shift left with overflow",
				BinOp::Shr => "attempt to shift right with overflow",
				_ => return self.err(span, format!("the assertion `{msg:?}`")),
			},
			AssertKind::OverflowNeg(_) => "attempt to negate with overflow",
			AssertKind::DivisionByZero(_) => "attempt to divide by zero",
			AssertKind::RemainderByZero(_) => "attempt to calculate the remainder with a divisor of zero",
			_ => return self.err(span, format!("the assertion `{msg:?}`")),
		};
		format!("rustPanic({text:?})")
	}

	fn call(&mut self, func: &Operand<'tcx>, args: &[&Operand<'tcx>], dest: Place<'tcx>, span: Span, out: &mut String) {
		let tcx = self.tcx;
		let dest_ty = dest.ty(self.mir, tcx).ty;
		let func_ty = self.operand_ty(func);
		let (callee, spread) = match *func_ty.kind() {
			ty::FnDef(def_id, gargs) => {
				let instance = match Instance::try_resolve(tcx, self.t.env(), def_id, gargs.skip_binder()) {
					Ok(Some(instance)) => instance,
					_ => {
						self.err(
							span,
							format!("a call of `{}` that does not resolve", tcx.def_path_str(def_id)),
						);
						return;
					}
				};
				if let Some(text) = library::call(self, instance, args, dest_ty, span) {
					let text = match text {
						library::Bound::Expr(e) if Translator::no_value(dest_ty) => e,
						library::Bound::Expr(e) => format!("{} = {e}", self.dest(dest, span)),
						library::Bound::Stmt(s) => s,
					};
					writeln!(out, "\t{text}").unwrap();
					return;
				}
				match instance.def {
					InstanceKind::Intrinsic(_) => {
						let name = tcx.item_name(instance.def_id());
						self.err(span, format!("the intrinsic `{name}`"));
						return;
					}
					InstanceKind::Virtual(..) => {
						self.err(span, "a call through a trait object");
						return;
					}
					InstanceKind::Item(_) if !tcx.is_mir_available(instance.def_id()) => {
						self.err(span, format!("`{instance}`, which has no MIR and no binding"));
						return;
					}
					_ => {}
				}
				let rust_call = tcx.fn_sig(def_id).skip_binder().abi() == rustc_abi::ExternAbi::RustCall;
				(self.t.func(instance, span), rust_call)
			}
			ty::FnPtr(..) => (self.operand(func, span), false),
			_ => {
				self.err(span, format!("a call of `{func_ty}`"));
				return;
			}
		};
		let mut values = Vec::new();
		for (i, arg) in args.iter().enumerate() {
			let v = self.operand(arg, span);
			if spread && i + 1 == args.len() {
				let ty::Tuple(tys) = self.operand_ty(arg).kind() else {
					self.err(span, "a rust-call whose last argument is no tuple");
					return;
				};
				values.extend((0..tys.len()).map(|f| format!("{v}.F{f}")));
			} else {
				values.push(v);
			}
		}
		let call = format!("{callee}({})", values.join(", "));
		match Translator::no_value(dest_ty) {
			true => writeln!(out, "\t{call}").unwrap(),
			false => {
				let dest = self.dest(dest, span);
				writeln!(out, "\t{dest} = {call}").unwrap()
			}
		}
	}
}

/// The width of an integer type, or a `char`'s or a `bool`'s, in bits.
pub fn int_bits<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> u32 {
	match ty.kind() {
		ty::Int(i) => i.bit_width().unwrap_or(tcx.data_layout.pointer_size().bits()) as u32,
		ty::Uint(u) => u.bit_width().unwrap_or(tcx.data_layout.pointer_size().bits()) as u32,
		ty::Char => 32,
		ty::Bool => 8,
		_ => 64,
	}
}
