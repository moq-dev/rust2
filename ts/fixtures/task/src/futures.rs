//! std's future helpers over a channel: `pin!`, `poll_fn`, `future::pending`, `Poll`'s
//! methods, and wakers from `Arc<impl Wake>` and `Waker::noop`, and their identities; and
//! boxed futures, `Pin<Box<dyn Future>>` included.

use std::cell::Cell;
use std::future::{self, Future};
use std::pin::{Pin, pin};
use std::ptr;
use std::sync::Arc;
use std::task::{Context, Poll, RawWakerVTable, Wake, Waker};

use kio::Consumer;

use crate::log::Log;
use crate::model::Guard;

/// Every value until the channel closes, summed through std's helpers: a pinned `recv`
/// polled by hand from a `poll_fn` that owns a guard, which a cancel drops with the
/// `poll_fn`; a pinned `future::pending` polled first with `Waker::noop`, which never
/// wakes; and `Poll`'s methods on what each poll returns. `recv` wakes the task through
/// a relay, a waker the `poll_fn` makes on its first poll from an `Arc` of a `Wake` impl,
/// which an even value also wakes by value, and which logs its own drop. It stops at a
/// value of 77. First, a `poll_fn` of a borrowed function drops in an `Arc` without
/// dropping the function.
pub async fn helpers(mut rx: Consumer, guard: Guard) -> u64 {
	let lent = Guard::new(guard.id() + 900, guard.log().clone());
	let mut lend = move |_: &mut Context<'_>| Poll::Ready(lent.id());
	drop(Arc::new(future::poll_fn(&mut lend)));
	guard.note("lent");
	drop(lend);
	let mut total = 0;
	loop {
		let owned = Guard::new(guard.id() + 500, guard.log().clone());
		let mut never = pin!(future::pending::<u64>());
		let mut next = pin!(rx.recv());
		let mut relay: Option<Waker> = None;
		let read = future::poll_fn(move |cx| {
			let waiting = never.as_mut().poll(&mut Context::from_waker(Waker::noop()));
			if relay.is_none() {
				relay = Some(relayed(cx.waker(), owned.log().clone(), owned.id() + 100));
			}
			let relay = relay.as_ref().unwrap();
			let polled = next.as_mut().poll(&mut Context::from_waker(relay));
			owned.note(&format!(
				"pending {} ready {} pending {}",
				waiting.is_pending(),
				polled.is_ready(),
				polled.is_pending()
			));
			if let Poll::Ready(Some(value)) = polled
				&& value % 2 == 0
			{
				relay.clone().wake();
			}
			polled
				.map(|value| value.ok_or(total))
				.map_ok(|value| value * 2)
				.map_err(|total| total + 1)
		})
		.await;
		match read {
			Ok(154) => return total,
			Ok(value) => total += value,
			Err(end) => return end,
		}
	}
}

/// A waker relaying to `task`, through a `Wake` impl with its own `wake_by_ref` for an
/// even `id`, and `Wake`'s default otherwise.
fn relayed(task: &Waker, log: Log, id: u32) -> Waker {
	let relay = Relay {
		task: task.clone(),
		guard: Guard::new(id, log),
	};
	match id % 2 {
		0 => Waker::from(Arc::new(relay)),
		_ => Waker::from(Arc::new(Plain(relay))),
	}
}

/// Wakes `task`, logging through `guard`, which logs the relay's drop.
struct Relay {
	task: Waker,
	guard: Guard,
}

impl Wake for Relay {
	fn wake(self: Arc<Self>) {
		self.guard.note("relay wake");
		self.task.wake_by_ref();
	}

	fn wake_by_ref(self: &Arc<Self>) {
		self.guard.note("relay wake_by_ref");
		self.task.wake_by_ref();
	}
}

/// A relay without its own `wake_by_ref`.
struct Plain(Relay);

impl Wake for Plain {
	fn wake(self: Arc<Self>) {
		self.0.guard.note("plain wake");
		self.0.task.wake_by_ref();
	}
}

/// A waker's identity, as kio's `Polling` keeps it.
type Identity = (*const (), *const RawWakerVTable);

std::thread_local! {
	/// The identity of the waker of the innermost `run` on this thread.
	static POLLING: Cell<Option<Identity>> = const { Cell::new(None) };
}

/// Run `poll` as the task `waker` wakes, as kio's `Polling::run` does.
fn run<R>(waker: &Waker, poll: impl FnOnce() -> R) -> R {
	let outer = POLLING.replace(Some((waker.data(), waker.vtable())));
	let output = poll();
	POLLING.set(outer);
	output
}

/// Whether `waker` is the one the innermost `run` holds, as kio's `Polling::hold` asks.
fn polling(waker: &Waker) -> bool {
	POLLING.with(|polling| polling.get() == Some((waker.data(), waker.vtable())))
}

/// Every value until the channel closes, summed, comparing wakers' identities as kio's
/// `Polling` does. First, wakers made from one `Arc` and from another, of two `Wake`
/// impls, `Waker::noop`, and pointers to `()`; then, each poll, the task's waker against
/// its last poll's, a clone, a `Context` of a clone, and a `run` of `Waker::noop` inside.
/// Not a clone of an `Arc`'s waker or of `Waker::noop` against its original: the clone's
/// vtable is another function's constant, whose address is the same only when the build
/// puts both in one codegen unit, as a non-incremental build does and an incremental one
/// may not. The executor's vtable is a `static`, so its clones compare reliably.
pub async fn wakers(mut rx: Consumer, guard: Guard) -> u64 {
	let relay = |id| Relay {
		task: Waker::noop().clone(),
		guard: Guard::new(guard.id() + id, guard.log().clone()),
	};
	let one = Arc::new(relay(100));
	let (a, b) = (Waker::from(one.clone()), Waker::from(one.clone()));
	let other = Waker::from(Arc::new(relay(200)));
	let plain = Waker::from(Arc::new(Plain(relay(300))));
	let noop = Waker::noop();
	let unit = Box::into_raw(Box::new(()));
	let checks = [
		a.will_wake(&b),
		a.data() == b.data(),
		a.will_wake(&other),
		a.data() == other.data(),
		ptr::eq(a.vtable(), other.vtable()),
		ptr::eq(a.vtable(), plain.vtable()),
		a.will_wake(noop),
		noop.will_wake(Waker::noop()),
		noop.data().is_null(),
		ptr::null::<()>() == ptr::dangling(),
		ptr::dangling::<()>() == ptr::dangling(),
		unit.cast_const() == ptr::dangling(),
		ptr::dangling::<std::mem::MaybeUninit<()>>().cast::<()>() == ptr::dangling(),
	];
	// SAFETY: the box's own pointer, which nothing else holds.
	drop(unsafe { Box::from_raw(unit) });
	put(Box::leak(Box::new(())), ());
	// SAFETY: a dangling pointer to a value of no bytes is valid to write.
	put(unsafe { &mut *ptr::dangling_mut() }, ());
	guard.note(&format!("wakers {checks:?}"));
	drop((a, b, other, plain));
	let mut last: Option<Identity> = None;
	let mut total = 0;
	loop {
		let mut next = pin!(rx.recv());
		let value = future::poll_fn(|cx| {
			let waker = cx.waker();
			let same = last == Some((waker.data(), waker.vtable()));
			last = Some((waker.data(), waker.vtable()));
			run(waker, || {
				let clone = waker.clone();
				let lent = Context::from_waker(&clone);
				let inner = run(noop, || (polling(noop), polling(waker)));
				guard.note(&format!(
					"same {same} clone {} lent {} inner {inner:?} after {}",
					polling(&clone),
					lent.waker().will_wake(waker),
					polling(waker),
				));
				next.as_mut().poll(cx)
			})
		})
		.await;
		match value {
			Some(value) => total += value,
			None => return total,
		}
	}
}

/// Write `value` through `slot`, which a dangling pointer to `()` may give.
fn put<T>(slot: &mut T, value: T) {
	*slot = value;
}

/// A boxed future, as moq's `MaybeSendBox` is.
type Boxed<'a> = Pin<Box<dyn Future<Output = u64> + 'a>>;

/// Boxed futures over a channel: `Box::pin` of an async fn, awaited, and of a `&mut` to a
/// `poll_fn` and to a `&mut` to it, whose box drops without dropping it, and a `&mut dyn
/// Future` of a `&mut` to it, then a queue of
/// `Pin<Box<dyn Future>>`s of an async block, kio's `Pending`, a struct of the crate's,
/// and a `poll_fn`, each owning a guard. A `poll_fn` boxed in a [`Forward`] polls the
/// queue in turn through [`poll_unsized`], as moq's tasks do through kio's
/// `Waiter::poll_future`, dropping each future as it is ready, and a pinned `&mut dyn
/// Future` of the crate's struct last. A cancel drops the futures the queue still holds,
/// in order.
pub async fn boxed(mut rx: Consumer, guard: Guard) -> u64 {
	let first = Box::pin(rx.recv()).await.unwrap_or(0);
	guard.note(&format!("first {first}"));
	let (id, log) = (guard.id(), guard.log().clone());
	let lent = Guard::new(id + 900, log.clone());
	let mut lend = future::poll_fn(move |_| {
		lent.note("lent");
		Poll::Ready(4)
	});
	let erased: Boxed<'_> = Box::pin(&mut lend);
	let lent = erased.await;
	let mut lending = &mut lend;
	let twice: Boxed<'_> = Box::pin(&mut lending);
	let lent = lent + twice.await;
	let mut lending = &mut lend;
	let direct: &mut (dyn Future<Output = u64> + Unpin) = &mut lending;
	let lent = lent + direct.await + (&mut lend).await;
	guard.note(&format!("lent {lent}"));
	drop(lend);
	let (block, waiting, ready) = (
		Guard::new(id + 500, log.clone()),
		Guard::new(id + 600, log.clone()),
		Guard::new(id + 700, log.clone()),
	);
	let mut next = rx.clone();
	let mut queue: Vec<Boxed> = vec![
		Box::pin(async move {
			let value = rx.recv().await.unwrap_or(0);
			block.note(&format!("block {value}"));
			value + 1
		}),
		Box::pin(kio::wait(move |waiter| {
			next.poll_recv(waiter).map(|value| {
				waiting.note("waited");
				value.unwrap_or(0) + 2
			})
		})),
		Box::pin(future::poll_fn(move |_| {
			ready.note("ready");
			Poll::Ready(3)
		})),
	];
	let tail = Guard::new(id + 800, log.clone());
	let mut last = pin!(kio::wait(move |_| Poll::Ready(u64::from(tail.id()))));
	let mut total = 0;
	let forward = Forward {
		future: Box::pin(future::poll_fn(move |cx| {
			let mut i = 0;
			while i < queue.len() {
				match poll_unsized(queue[i].as_mut(), cx) {
					Poll::Ready(value) => {
						total += value;
						drop(queue.swap_remove(i));
					}
					Poll::Pending => i += 1,
				}
			}
			if !queue.is_empty() {
				return Poll::Pending;
			}
			let last: Pin<&mut dyn Future<Output = u64>> = last.as_mut();
			poll_unsized(last, cx).map(|value| total + value)
		})),
	};
	let total = forward.await;
	guard.note(&format!("boxed {total}"));
	total
}

/// Polls `future`, sized or not, as kio's `Waiter::poll_future` does.
fn poll_unsized<F: Future + ?Sized>(future: Pin<&mut F>, cx: &mut Context<'_>) -> Poll<F::Output> {
	future.poll(cx)
}

/// A future of the crate's that forwards its polls to a boxed one, as moq's session
/// driver does.
struct Forward<'a> {
	future: Boxed<'a>,
}

impl Future for Forward<'_> {
	type Output = u64;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u64> {
		self.future.as_mut().poll(cx)
	}
}
