//! Async closures over a channel: called directly, as moq-net's tests call them, and
//! through `AsyncFn`, `AsyncFnMut`, `AsyncFnOnce`, and `Fn`, each borrowing what it
//! captured or owning it.

use std::future::Future;

use kio::Consumer;

use crate::model::Guard;

/// Values read from the channel through async closures, summed. A closure that borrows
/// the channel, a list, and a count mutably is called directly and through `AsyncFnMut`;
/// one that owns a count changes its own; an owning one, called through `AsyncFnOnce`,
/// moves its guard into its future, which drops it as it returns or is cancelled; one
/// that only borrows is called through `AsyncFn` and through `Fn`, a future of its own
/// each call, and so is one that captures nothing; one that moves its guard out is called
/// once; one made by a function holds a copy of its argument; and one that owns a guard,
/// called through `AsyncFn`, is then consumed through `FnOnce`. A clone of the one that owns a count changes its own. A
/// cancel drops what the future in flight holds, then the closures, in reverse.
pub async fn closures(mut rx: Consumer, guard: Guard) -> u64 {
	let (id, log) = (guard.id(), guard.log().clone());
	let mut seen = Vec::new();
	let mut reads = 0;
	let mut next = async |scale: u64| {
		let value = rx.recv().await;
		seen.push(value);
		reads += 1;
		value.map(|value| value * scale)
	};
	let first = next(1).await;
	let second = twice(&mut next).await;
	let mut count = reads;
	let mut tally = async move |by: u32| {
		count += by;
		count
	};
	let once = tally(10).await;
	let mut copy = tally.clone();
	let tallied = (once, copy(100).await, tally(1000).await);
	guard.note(&format!(
		"seen {seen:?} first {first:?} second {second:?} reads {reads} tallied {tallied:?} count {count}"
	));
	let owned = Guard::new(id + 500, log.clone());
	let once = async move |rx: &mut Consumer| {
		let value = rx.recv().await;
		owned.note(&format!("once {value:?}"));
		value.unwrap_or(0)
	};
	let third = call_once(once, &mut rx).await;
	guard.note(&format!("third {third}"));
	let lent = Guard::new(id + 600, log.clone());
	let peek = async |value: u64| {
		lent.note(&format!("peek {value}"));
		value + 1
	};
	let fourth = call(&peek, 4).await + through_fn(&peek, 5).await + through_fn(async |value| value * 3, 6).await;
	let moved = Guard::new(id + 700, log.clone());
	let consume = async move || {
		let id = moved.id();
		drop(moved);
		id
	};
	let add = adder(u64::from(consume().await));
	let last = Guard::new(id + 800, log.clone());
	let finish = async move |value: u64| {
		last.note(&format!("last {value}"));
		value
	};
	let fifth =
		call(&add, 10).await + call(&finish, 20).await + through_fn_once(finish, rx.recv().await.unwrap_or(0)).await;
	guard.note(&format!("fourth {fourth} fifth {fifth}"));
	first.unwrap_or(0) + second.unwrap_or(0) + third + fourth + fifth
}

/// Two calls of `f`, through `AsyncFnMut`.
async fn twice(f: &mut impl AsyncFnMut(u64) -> Option<u64>) -> Option<u64> {
	Some(f(2).await? + f(3).await?)
}

/// One call of `f`, through `AsyncFnOnce`.
async fn call_once(f: impl AsyncFnOnce(&mut Consumer) -> u64, rx: &mut Consumer) -> u64 {
	f(rx).await
}

/// One call of `f`, through `AsyncFn`.
async fn call(f: &impl AsyncFn(u64) -> u64, value: u64) -> u64 {
	f(value).await
}

/// One call of `f`, through `Fn` returning a future.
async fn through_fn<F: Fn(u64) -> R, R: Future<Output = u64>>(f: F, value: u64) -> u64 {
	f(value).await
}

/// One call of `f`, through `FnOnce` returning a future.
async fn through_fn_once<F: FnOnce(u64) -> R, R: Future<Output = u64>>(f: F, value: u64) -> u64 {
	f(value).await
}

/// An async closure adding `n`.
fn adder(n: u64) -> impl AsyncFn(u64) -> u64 {
	async move |value| value + n
}
