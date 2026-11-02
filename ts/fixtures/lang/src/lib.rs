//! The language the driver translates, exercised against native Rust: every integer
//! width's arithmetic, casts, and inherent methods, `f64`, `char`s and the `str` methods over them,
//! `Cow<str>`, boxed slices and text, `Duration`, control flow, patterns, byte slices split apart,
//! passed as their parts, returned in a slot, and held as their parts in fields, `Copy`
//! aggregates, closures and what they write of what they captured, traits, generic
//! types, formatting, collections, `LinkedList`s, UTF-8 decoding, `&mut` to values JS copies, whole
//! writes through a `&mut` to an object, the standard library calls it maps, `Option`'s
//! and `Result`'s combinators from core's MIR, `Result`'s iterators, `Option`s of values that can already be
//! `undefined`, atomics, `static` ones included, trait objects, one-time cells, mutexes,
//! `Cell`s, `UnsafeCell`s, `RefCell`s, read-write locks, and thread-locals, the defaults
//! of std types, the counts of shared pointers, arrays `array::from_fn` and `map` make,
//! their elements each lent, arrays as slices, maps of `Option`s, values held back past a
//! write of a variable they name, what
//! `Hash` writes into `DefaultHasher` and `RandomState`, slices' `windows` and chunks,
//! mutable ones written through, their parts by inclusive and checked ranges and by fixed
//! size, slices split by a predicate, `Vec::drain`, `concat` and `join` of slices of
//! slices, the order, equality, and
//! hashes of bytes, ranges of `Bound`s and of the crate's own `RangeBounds`, the equality
//! of `Result`s and std's errors, inclusive ranges read through each adaptor, iterators'
//! size hints, the capacity overflows collections reserving by them panic with, the
//! capacity of a `Vec`, `VecDeque`, and `BinaryHeap` as each grows and shrinks, pointers
//! into arrays moved, measured, and copied between, tuples of
//! collections collected and extended by their items, zero-sized values, which MIR
//! never assigns, with std's impls of them, and functions generic over an array's length.
//!
//! `harness::run` is the entry point both sides of the differential test run, and
//! `fmt::Fault` an error only `caller.ts` prints.

#![feature(
	cell_leak,
	iter_order_by,
	mapped_lock_guards,
	once_cell_try,
	refcell_try_map,
	split_as_slice,
	wrapping_int_impl,
	wrapping_next_power_of_two
)]

mod args;
mod arrays;
mod ascii;
mod atomics;
mod borrows;
mod bounds;
mod boxes;
mod capacity;
mod captures;
mod cells;
mod chars;
mod chunks;
mod clones;
mod collections;
mod combinators;
mod counts;
mod cows;
mod cycles;
mod defaults;
mod durations;
mod dyns;
mod edits;
mod elements;
mod errors;
mod fields;
mod floats;
mod fmt;
mod folds;
mod generics;
pub mod harness;
mod hashes;
mod hints;
mod holds;
mod ints;
mod iters;
mod joins;
mod keys;
mod lengths;
mod lists;
mod locals;
mod locks;
mod methods;
mod nested;
mod objects;
mod orders;
mod pointers;
mod refs;
mod results;
mod returns;
mod saturates;
mod separated;
mod shapes;
mod singles;
mod slices;
mod splits;
mod statics;
mod strs;
mod tuples;
mod utf8;
#[cfg(target_arch = "wasm32")]
#[path = "../../wasm.rs"]
mod wasm;
mod wrappers;
mod wraps;
mod zeros;
#[cfg(target_arch = "wasm32")]
use harness::entry;
