//! moq-net's sync codec, as rust2-ts's codec fixture vendors it, translated to Go: its
//! sources are that fixture's, and only `bench` is this crate's own, the workloads of
//! that fixture's benchmark that code moq's wire format.

#![allow(dead_code, reason = "a vendored subset of moq-net, which uses the rest")]
#![allow(
	clippy::allow_attributes_without_reason,
	reason = "moq-net's sources, which rust2-ts's fixture builds with `async` where this is reached"
)]

pub mod bench;
#[path = "../../../../ts/fixtures/codec/src/coding/mod.rs"]
mod coding;
#[path = "../../../../ts/fixtures/codec/src/harness.rs"]
pub mod harness;
#[path = "../../../../ts/fixtures/codec/src/ietf"]
mod ietf {
	mod version;
	pub use version::Version;
}
#[path = "../../../../ts/fixtures/codec/src/lite"]
mod lite {
	mod group;
	pub mod message;
	mod probe;
	mod version;
	pub use group::*;
	pub use message::Message;
	pub use probe::*;
	pub use version::Version;
}
#[path = "../../../../ts/fixtures/codec/src/version.rs"]
mod version;

pub use version::*;

pub mod stats {
	/// The control message classes moq-net's metrics count, trimmed to what the codec names.
	#[derive(Clone, Copy, Debug, PartialEq, Eq)]
	#[non_exhaustive]
	pub enum Control {
		/// Bandwidth probes.
		Probe,
		/// Other protocol control messages.
		Other,
	}
}

/// moq-net's session error, trimmed to a few variants: the codec only carries it.
#[derive(thiserror::Error, Debug, Clone)]
#[non_exhaustive]
pub enum SessionError {
	#[error("protocol violation")]
	ProtocolViolation,
	#[error("app code={0}")]
	App(u16),
}

/// moq-net's track timescale, stubbed for `lite::Version::wire_timescale`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timescale(u64);

impl Timescale {
	/// One unit per millisecond.
	pub const MILLI: Self = Self(1_000);
}
