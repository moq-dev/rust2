# [M] A crate's own hasher

## Goal

[rust2-ts](/ts/README.md) translates hashing through a crate's own `Hasher` and
`BuildHasher`, as `hashlink`'s default `DefaultHashBuilder` wraps hashbrown's: std's
provided `BuildHasher::hash_one`, and std's `Hash` impls writing into that hasher with the
calls std makes, proven by differential tests against native Rust.

## Plan

Found by [#398](https://github.com/moq-dev/rust2/pull/398), which maps hashbrown's
`DefaultHashBuilder` and `DefaultHasher` to std's: a `LinkedHashMap::new()` then refuses
with "unsupported call to `<hashlink::DefaultHashBuilder as
std::hash::BuildHasher>::hash_one::<&u16>`". moq's model reaches it through hashlink's
default hasher, the census's last refusals in `hashlink/src/lib.rs`.

- `hash_one` is `build_hasher`, then `Hash::hash`, then `finish`, each the crate's.
- std's `Hash` of an integer, `str`, slice, tuple, and the rest calls the hasher's
  `write_*` methods in std's order, length prefixes and `str`'s `0xff` included, so a
  hasher that reads its input, not only hashlink's forwarding one, sees what native's does.
- Today only std's `DefaultHasher` takes std's `Hash` impls, written by the runtime's
  functions.

Public API: the translator's output.
