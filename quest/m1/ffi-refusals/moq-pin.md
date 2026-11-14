# [S] Bump the exporter's moq pin past its re-export and reachability fixes

## Goal

The exporter's moq pin moves to moq's current `main`, whose crates re-export no whole crate, moq's own
included. `just ffi census` there lists no whole-crate re-export refusal. Its "type not
reachable from the crate root" refusals were `kio::Pending` and moq-tokio's `cli::Deprecated`,
which the exporter change below exports, `lite::version::Version`, which no public path names, and moq-quic's stats and varint types under moq-tokio's `quic::driver`,
which [kixel-ai/rust2#683](https://github.com/kixel-ai/rust2/pull/683) drops. At `ffcb7cd9` there were 20 such refusals (18 Guard and Side, and
`Version`), and 9 at moq's head on 2026-11-09, measured in
[#676](https://github.com/kixel-ai/rust2/pull/676).

## Plan

Decided with the maintainer on 2026-11-07, from
[kixel-ai/rust2#655](https://github.com/kixel-ai/rust2/pull/655) and
[moq-dev/moq#8233](https://github.com/moq-dev/moq/pull/8233): ✅ moq re-exports no whole
crate, its own included: the ten first listed (moq-net's `bytes` and `kio`; `rustls` in
moq-quic, moq-rtmp, and moq-tokio; moq-tokio's `notify`, `usage_rs`, and `http`;
moq-uring's `http`; moq-mux's `mp4_atom`), hang's and moq-tokio's `pub use moq_net;`,
moq-tokio's `moq_congestion as congestion`, `web_transport_proto as proto` in moq-tokio and
moq-uring, and the rest, such as `axum`, `url`, `object_store`, moq-video's platform crates,
`jni`, and `tikv_jemallocator`. moq-quic-unused-surface names the moq-quic items it needs
instead of adding `pub use moq_quic;`. Each whole-crate re-export makes that crate's whole
API part of moq's, so any breaking release of it breaks moq's semver; moq re-exports by
name only the items its signatures need, or wraps them. Rejected: only the ten; only
third-party crates.

At `ffcb7cd9`, `container::Producer::modify` returns moq-mux's
`container::producer::Guard`, in a private module (16 refusals), and moq-tokio's
`quic::driver` re-exports moq-quic's `stream::Id` but not the `Side` its `new` and
`initiator` take (2). Both are moq's to fix; the stand-in's narrowing to moq-tokio's
session side also drops `quic::driver`.

Decided with the maintainer on 2026-11-07, in the quest audit: one pin bump for both, since
each is an XS bump of the same pin line. Rejected: one bump per moq quest. Superseded on
2026-11-12 by the split below.

Both of moq's fixes merged on 2026-11-09:
[moq-dev/moq#8247](https://github.com/moq-dev/moq/pull/8247) dropped every whole-crate
re-export and re-exports `Guard`, and
[moq-dev/moq#8302](https://github.com/moq-dev/moq/pull/8302) removed
`stream::Id::{new, initiator}`. At moq's head the 9 unreachable items left are
`kio::Pending` (from `dial_tcp` and `dial_track`, reachable before only through
`pub use kio;`), `lite::version::Version`, which was already unreachable at the pin, and
moq-tokio's `quic::driver` items, which the narrowed stand-in drops.

Then bump the exporter's `moq_rev` in `ffi/justfile`, and report the census before and
after. The last bump ([#390](https://github.com/kixel-ai/rust2/pull/390)) also changed
`ffi/moq/ffi.toml`, every `ffi/moq/moq.*`, and rust2-ts's model fixture, bench, and expected
output, which then shared one pin. rust2-ts's pin moves in
[its own quest](/quest/m2/ts-model/moq-pin.md), so its census count is updated there.

Decided with the maintainer on 2026-11-09: bump to moq's current main now, and narrow the
Goal to the two moq-owned types. Rejected: wait for moq to fix the rest first. Not landed:
moq's `main` fails `just ts moq` and `just ffi moq-cpp`, so the bump waits on Waiting. The
split on 2026-11-12 moved the `just ts moq` failure to
[its own quest](/quest/m2/ts-model/moq-pin.md), so this bump waits on `just ffi moq-cpp` alone.

Decided 2026-11-11 (maintainer, /quest-unblock): ✅ (a) rust2 exports a type the crate root does
not reach by its defining path, such as `kio::Pending` and `lite::version::Version`; a type
whose defining path is itself private stays refused (rejected: (b) moq makes both reachable).
[moq-dev/rust2#721](https://github.com/moq-dev/rust2/pull/721) made the exporter change: a
type a signature names that the root does not reach exports by its shortest public path in
its own crate, so `kio::Pending` (defined in kio's private `pollable`) and moq-tokio's
`cli::Deprecated` export, the glue naming them through its own dependency on their crate's
package. At moq `ba47ea3` the census, which ignores `ffi/moq/ffi.toml`'s excludes, then lists
no such refusal; only `lite::version::Version`, which no public path names, stays out, as an
object no export returns. The bump drops the stale `kio` exclude, which would otherwise leave
`kio::Pending` out of the export.

Decided 2026-11-13 (maintainer): ✅ widen #721 to a type a signature names from a dependency
the facade exports nothing else from, such as `pub fn pending() -> kio::Pending` with no
`pub use kio::…` (rejected: land as is with a follow-up quest; land with no follow-up). Such
a crate keeps the roles of one no export is: an error of it never exports (one in a `Result`
crosses as its message, one in an exported error's variant leaves that error an object), and
its type behind an `Arc` stays an opaque handle, refused bare. At the pin this reaches
mp4_atom's boxes (through moq-mux's `container::fmp4::Wire::new`, `Wire::trak`, and
`fmp4::Error`), moq-quic, moq-sock, and rustls, so `ffi/moq/ffi.toml` excludes only what
they still refuse.

Decided 2026-11-13 (maintainer, #721's design checkpoint): ✅ an `impl Trait` return holds
what its hidden type holds, as a named return does, so `fn wrap(x: &u32) -> impl Clone + '_ {
Some(Held(x)) }` is refused instead of producing glue that does not build, while an iterator
collected at the call still exports (rejected: map every `impl Trait` return as its hidden
type everywhere; land as is with a follow-up quest). Implementing it showed the glue keeps the
`impl Trait` itself, never its hidden type, so #721 lets one cross only when its hidden type is
an object, holding every lifetime it captures, and refuses any other. moq's census at the pin
is unchanged by it.

Decided with the maintainer on 2026-11-12, when the exporter's pin was split off: ✅ split the
bump, the exporter's pin here and rust2-ts's in [its own quest](/quest/m2/ts-model/moq-pin.md),
so this bump no longer waits on rind building for wasm32. Rejected: keep one bump for both
pins, which holds the exporter behind rind.

Measured on 2026-11-12 at moq's `main` `6936f0a`: `just ffi moq-cpp` fails the export with
36 errors, which the bump fixes in `ffi/moq/ffi.toml` or the exporter: 15 on the foreign
type `std::num::NonZero` (such as `Client::with_peer_hop`), 11 stale excludes naming nothing
exported (such as `bytes` and `kio`), and single refusals such as `qmux::Session`,
`CertificateDer`, a borrowing future from the non-async `Server::accept`, and borrowed
returns that are not `Clone`. At `9b80cfcc8` on 2026-11-14 it fails with 19: 15 stale
excludes, `dial_tcp` and `dial_track` on the excluded `kio::Pending`, and two rustls
`ConfigBuilder` instances on the foreign `ServerConfig`.

Decided 2026-11-14 (maintainer, [kixel-ai/rust2#809](https://github.com/kixel-ai/rust2/pull/809)):
✅ keep waiting on moq, and plan the fix in moq-dev/moq: wrap the lite publisher's
`kio::Tasks<Control>` so it proves `Send + Sync` once, as
[moq-dev/moq#8657](https://github.com/moq-dev/moq/pull/8657) did for the subscriber, and build
moq-tokio and the moq-ffi facade under `-D recursion_depth_exceeding_limit` so the overflow
cannot return. Rejected: record the wait only, with no moq fix planned; allow the lint for moq's
crates, which only defers the failure until a later nightly makes it a hard error.

## Waiting

- moq-dev/moq: moq-tokio builds without tripping `recursion_depth_exceeding_limit` (an
  overflow proving `Send` on `Instrumented<...>`), which `just ffi moq-cpp <checkout>` denies.
  Done when it passes at moq's `main`. Still failing on 2026-11-14 at moq `9b80cfcc8`, after
  [moq-dev/moq#8657](https://github.com/moq-dev/moq/pull/8657), which shortened only moq-net's
  own `Driver: Send` proof: at `rs/moq-tokio/src/client.rs:789`, on the `Instrumented<...>` from
  `tokio::spawn(panic::isolate(time::run(driver)).instrument(..))`, and in the moq-ffi facade
  on `moq_net::Driver: Sync`, through the lite session driver's `Publisher` and its
  `kio::Tasks<Control>`.
