# [S] Dropping a crate's generic type inside another

## Goal

[rust2-ts](/ts/README.md) drops a crate's generic type held in a field of another of the
crate's generic types, as `hashlink`'s `LruCache<K, V, S>` holds a `LinkedHashMap<K, V,
S>`, in the order rustc's glue does, proven by differential tests against native Rust.

## Plan

Found by [hashbrown as the runtime's map](https://github.com/moq-dev/rust2/pull/398): a
fixture dropping an `LruCache` refuses with "dropping `hashlink::LinkedHashMap<K, V, S>`,
which is not supported yet" at the end of the function holding it, from
`drop_glue_call`, which takes the generic field's glue only in a census. A
`LinkedHashMap` dropped by itself translates, so the outer glue's shim is what lowers the
field over its type parameters rather than the instance. `LinkedHashSet` is expected to
refuse the same way. No consumer calls them yet; they are planned here as part of
covering stock crates.

Public API: the translator's output.
