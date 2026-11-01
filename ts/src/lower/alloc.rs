//! `std::alloc`, as `rt/alloc.ts` gives it: a `Layout` is its size and alignment, never
//! changed once made, and memory from `alloc` is untyped until a cast of its pointer to one
//! to `T`, which [`crate::lower::mir`] allows only where the body shows the pointer came
//! from `alloc` or `realloc`, makes it a JS array of `T`s. A pointer to it as bytes is
//! allowed only as what `dealloc` or `realloc` takes.

use super::body::Body;
use super::library::Call;
use crate::ts::{Expr, Key};

/// A call of a `Layout` or allocator function, or its arguments back if it is none of theirs.
pub fn call<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	owner: &str,
	name: &str,
	args: Vec<Expr>,
) -> Result<Option<Expr>, Vec<Expr>> {
	let layout = |b: &mut Body<'_, '_, 'tcx>| {
		let ty = call.args.types().next().expect("a type argument");
		super::iter::layout(b, ty, call.span)
	};
	let mut args = args.into_iter();
	let mut arg = || args.next().expect("an argument");
	let e = match (owner, name) {
		("Layout", "new") => match layout(b) {
			Ok((size, align)) => object(Expr::lit(size.to_string()), Expr::lit(align.to_string())),
			Err(e) => e,
		},
		("Layout", "array") => match layout(b) {
			Ok((size, align)) => Expr::rt(
				"layoutArray",
				vec![arg(), Expr::lit(size.to_string()), Expr::lit(align.to_string())],
			),
			Err(e) => e,
		},
		("Layout", "from_size_align") => Expr::rt("layoutFromSizeAlign", vec![arg(), arg()]),
		("Layout", "from_size_align_unchecked") => object(arg(), arg()),
		("Layout", "size" | "align") => arg().member(name),
		("alloc", "alloc") => Expr::rt("alloc", vec![arg()]),
		("alloc", "dealloc") => Expr::rt("dealloc", vec![arg()]),
		("alloc", "realloc") => {
			let (p, _, size) = (arg(), arg(), arg());
			Expr::rt("realloc", vec![p, size])
		}
		("alloc", "handle_alloc_error") => Expr::rt("handleAllocError", vec![arg()]),
		_ => return Err(args.collect()),
	};
	Ok(Some(e))
}

/// A `Layout` of `size` bytes aligned to `align`.
pub fn object(size: Expr, align: Expr) -> Expr {
	Expr::Object(vec![
		(Key::Name("size".into()), size),
		(Key::Name("align".into()), align),
	])
}
