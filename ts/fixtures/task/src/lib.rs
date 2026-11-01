//! A kio-shaped channel and the async code moq builds on it, exercised against native
//! Rust: laziness, poll and wake order, and every drop, including a future's on cancel.
//!
//! `drops` holds values that drop in what std holds them in.
//!
//! The `kio` dependency trims moq's kio to what its futures hinge on: a `Waiter` holding
//! the task's waker behind an `Arc` that `WaiterList`s hold weakly, `Task`, `Pending`,
//! `wait`, and a channel whose last producer closes it on drop. `model` holds the async
//! functions, `futures` one over std's future helpers, and `closures` one over async
//! closures. `exec` is a deterministic FIFO executor that logs every
//! poll, wake, and waker clone and drop, and `harness` runs a script of executor steps
//! through it; `tests/task.ts` runs the same script on the translation with its own
//! executor. `bench` holds the handoff `just ts bench` times. None of `exec`, `harness`,
//! and `bench` is translated.

#![feature(iter_order_by)]

pub mod bench;
pub mod closures;
pub mod drops;
pub mod exec;
pub mod futures;
pub mod harness;
pub mod log;
pub mod model;
#[cfg(target_arch = "wasm32")]
#[path = "../../wasm.rs"]
mod wasm;
/// What the wasm32 build exports: the benchmark's handoff with `bench`, else the harness.
#[cfg(all(target_arch = "wasm32", feature = "bench"))]
use bench::run as entry;
#[cfg(all(target_arch = "wasm32", not(feature = "bench")))]
use harness::run as entry;
