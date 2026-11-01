//! What a `LinkedList` drops of the values it holds, and when: as it drops or clears, front
//! to back; as `pop_*`, `split_off`, `append`, and `extend` move them; what its iterators
//! leave, read from either end; and what `clone_from` writes over or pops.

use std::collections::LinkedList;

use super::Key;
use crate::log::Log;

/// The ids of a list's keys, front to back, apart from `id`.
fn ids(id: u32, l: &LinkedList<Key>) -> String {
	l.iter().map(|k| (k.id - id).to_string()).collect::<Vec<_>>().join(",")
}

/// Run scenario `arg % 8`.
pub(super) fn run(log: &Log, id: u32, arg: u8) {
	let key = |n: u32| Key::new(id + n, log.clone());
	let keys = |ns: &[u32]| ns.iter().map(|&n| key(n)).collect::<LinkedList<Key>>();
	match arg % 8 {
		// Pushed at both ends, popped from both, and the rest dropped with the list.
		0 => {
			let mut l = LinkedList::new();
			l.push_back(key(2));
			l.push_front(key(1));
			l.push_back(key(3));
			l.push_back(key(4));
			l.push_front(key(0));
			drop(l.pop_back());
			log.note("popped back".into());
			drop(l.pop_front());
			log.note(format!("left {}", ids(id, &l)));
		}
		// `clear` empties it first, then drops each front to back; so does `=`'s drop of
		// what it replaces.
		1 => {
			let mut l = keys(&[1, 2, 3]);
			l.clear();
			log.note(format!("cleared {}", l.len()));
			l.push_back(key(4));
			l = keys(&[5, 6]);
			log.note(format!("replaced {}", ids(id, &l)));
		}
		// `into_iter` read from both ends, then dropped with the rest, front to back.
		2 => {
			let mut it = keys(&[1, 2, 3, 4, 5, 6]).into_iter();
			drop(it.next_back());
			drop(it.next());
			drop(it.next_back());
			log.note(format!("len {}", it.len()));
			let skipped = it.nth(1).map(|k| k.id - id);
			log.note(format!("nth {skipped:?}"));
		}
		// `split_off` at 0, the middle, and the end, then each part dropped.
		3 => {
			let mut l = keys(&[1, 2, 3, 4, 5]);
			let back = l.split_off(3);
			let none = l.split_off(3);
			let all = l.split_off(0);
			log.note(format!(
				"split {} {} {} {}",
				ids(id, &l),
				ids(id, &back),
				ids(id, &none),
				ids(id, &all)
			));
			drop(all);
			log.note("front dropped".into());
			drop(back);
		}
		// `append` and `extend` by a list move every node, leaving the source empty, and
		// `extend` by an iterator pushes what it reads.
		4 => {
			let mut l = keys(&[1, 2]);
			let mut other = keys(&[3, 4]);
			l.append(&mut other);
			log.note(format!("appended {} {}", ids(id, &l), other.len()));
			other.push_back(key(5));
			l.extend(other);
			l.extend([key(6), key(7)]);
			let mut empty = LinkedList::new();
			empty.append(&mut l);
			log.note(format!("moved {} {}", ids(id, &empty), l.len()));
		}
		// `clone_from` of a shorter list: each key written over drops after its clone is
		// made, and the rest pop from the back.
		5 => {
			let mut l = keys(&[1, 2, 3, 4]);
			let source = keys(&[5, 6]);
			l.clone_from(&source);
			log.note(format!("cloned {}", ids(id, &l)));
			let longer = keys(&[7, 8, 9]);
			l.clone_from(&longer);
			log.note(format!("cloned {}", ids(id, &l)));
		}
		// `extract_if` takes out what it is read up to; dropped early, it leaves the rest, and
		// drops its closure.
		6 => {
			let mut l = keys(&[1, 2, 3, 4, 5, 6]);
			let taken: Vec<Key> = l.extract_if(|k| (k.id - id) % 2 == 0).collect();
			log.note(format!("taken {} left {}", taken.len(), ids(id, &l)));
			drop(taken);
			let first = l.extract_if(|k| k.id - id > 1).next();
			log.note(format!(
				"first {:?} left {}",
				first.as_ref().map(|k| k.id - id),
				ids(id, &l)
			));
			// A closure that owns a key drops with the iterator holding it.
			let held = key(9);
			let mut it = l.extract_if(move |k| {
				let held = &held;
				k.id > held.id
			});
			log.note(format!("none {}", it.next().is_none()));
			drop(it);
			log.note(format!("kept {}", ids(id, &l)));
		}
		// What folds of `into_iter` drop as they go, and a clone of one.
		_ => {
			let mut it = keys(&[1, 2, 3]).into_iter();
			drop(it.next());
			let copy = it.clone();
			log.note(format!("counted {}", it.count()));
			let last = copy.last().map(|k| k.id - id);
			log.note(format!("last {last:?}"));
		}
	}
}
