# Every public item exports

## Goal

[rust2-ffi](/ffi/README.md) exports every `pub` item of the moq facade it is given, or
leaves out and lists in the census what C can never hold, so `just ffi census` lists no
refusals at the moq pin and `ffi.toml` is deleted: the exporter reads no config, and an
item it cannot export fails the export with a span.

The epic ends with [the config's deletion](/quest/m1/ffi-refusals/no-config.md), its last
child.

## Plan

Decided with the maintainer on 2026-11-04: configless, and an unsupported public item
fails the export rather than being excluded. ✅ Split: the naming rule first, then this.
Rejected: one quest that waits for the census to reach zero.

Decided with the maintainer on 2026-11-07, from
[kixel-ai/rust2#655](https://github.com/kixel-ai/rust2/pull/655):

- ✅ A whole-crate re-export (`pub use rustls;`) is refused with one span at its `pub use`,
  and moq drops its re-exports. A whole-crate re-export makes that crate's whole API part
  of moq's, so any breaking release of it breaks moq's semver; moq wraps the few
  third-party types its signatures need instead. Rejected: skipping them silently, as
  rustdoc does; exporting their whole APIs.
- ✅ No exemption: the rule covers every crate, moq's own included (`pub use moq_net;`,
  `moq_congestion as congestion`), so moq drops those too, per
  [moq-dev/moq#8233](https://github.com/moq-dev/moq/pull/8233). Rejected: exempting the
  crates the facade already exports, or moq's own; either needs a list, which is config.
- ✅ The stand-in re-exports only the session side moq-ffi ships from moq-tokio. Rejected:
  all of moq-tokio, whose tokio QUIC driver, CLI helpers, worker groups, and listeners no
  binding calls; the sans-IO driver bindings use instead is
  [session-quic](/quest/m3/session-quic.md). Children leave the moq-tokio items
  outside that session side alone, since narrowing the stand-in drops them.
- Narrowing the stand-in was to land first; the 2026-11-09 audit dropped that order, and
  [kixel-ai/rust2#683](https://github.com/kixel-ai/rust2/pull/683) narrowed it.
- ✅ What C can never hold, an object no export returns or lends to a closure, or a generic
  type no export instantiates, is left out of the bindings and kept listed in the census.
  Rejected: refusing them, which would make moq add constructors it has no use for.
- ✅ This epic, with the children below ranked by what moq-ffi's hand-written wrapper still
  covers.

Decided with the maintainer on 2026-11-09, from
[kixel-ai/rust2#678](https://github.com/kixel-ai/rust2/pull/678):

- ✅ rust2 asks moq for no source changes: changing moq is a non-goal, so every refusal
  that waited on moq changing a signature gets an answer on rust2's side. moq's merged
  removal of its whole-crate re-exports stands, as moq drops them for its semver anyway.
  Rejected: refusing a shape and leaving moq to change its signature.
- ✅ A third-party type a signature still names behind an `Arc`, such as
  `Arc<rustls::ServerConfig>`, crosses as an opaque handle with none of its API, which
  passes back into moq's calls. Rejected: moq wrapping it in a type of its own.

The census at moq `ffcb7cd9` (2026-11-07) refused 1111 items, which `ffi/moq/ffi.toml`'s
347 excludes hide by naming whole modules. About 790 lay in crates moq re-exports whole,
which [#668](https://github.com/kixel-ai/rust2/pull/668) has since made one refusal each,
and [#671](https://github.com/kixel-ai/rust2/pull/671) moved what C can never hold out of
the refusals, so the figures below predate both.
The other 324 are moq's own, 107 of them in moq-tokio (61 under `quic`); each child gives
its construct's count, and an item counts under every construct it hits. The counts
include the moq-tokio items narrowing drops; each child measures again once it lands. 17 more refuse
only because a type they name is refused, and clear with it. After the stand-in narrows
([kixel-ai/rust2#683](https://github.com/kixel-ai/rust2/pull/683)), the census refuses 130
items, 41 of them in moq-tokio, and 6 types.

Decided with the maintainer on 2026-11-12, in the quest audit:

- ✅ The exporter pins moq on its own, apart from rust2-ts's pin, so its bump no longer
  waits on rind building for wasm32, which only the translator needs. Rejected: one pin for
  both, at the cost of two revisions to bump.
- The last per-construct child landed in
  [#710](https://github.com/kixel-ai/rust2/pull/710), yet the Goal is not reached: the
  2026-11-11 nightly census
  ([run 43038758163](https://github.com/kixel-ai/rust2/actions/runs/43038758163)) still
  refuses 94 items and 5 types at the pin, behind `ffi/moq/ffi.toml`'s then 100 excludes, and
  moq's `main` adds the 36 errors [the pin bump](/quest/m1/ffi-refusals/moq-pin.md)
  measured. So each construct left has a child again:
  `NonZero`, which exports, the rest of the errors at moq's main
  (exported since by rules for another crate's value, a borrowing future of a function
  that is not `async`, and an object field that is not `Clone`), and
  what the excludes still hide at moq's main, exported since but for
  [`IpAddr`](/quest/m1/ffi-refusals/ip-addr.md) and `server::Listener::accept_with`, which
  [not-send](/quest/m1/ffi-export/not-send.md) owns;
  `just ffi census` fails on an exclude that names what exports. The
  bump is then only the bump. The types refused for not being `Send` (`clock::Entered`,
  `clock::Rng`, `catalog::Guard`) clear with [not-send](/quest/m1/ffi-export/not-send.md).
- ✅ The closing work is a child, [no config](/quest/m1/ffi-refusals/no-config.md), as
  [the exporter epic's](/quest/m1/ffi-export/ship.md) is: an epic's `Required` would hold
  back every child, and this epic's end needs not-send, which is another epic's.

Public API: the exporter's command line and the C API of every item that exports.
