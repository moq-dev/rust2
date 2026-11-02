# Migrating

## Unreleased

- Every export writes `swift/<prefix>.swift`, whose runtime claims the C names
  `<prefix>_atomic_load`, `<prefix>_atomic_store`, `<prefix>_atomic_add`,
  `<prefix>_atomic_exchange`, `<prefix>_atomic_cas`, and `<PREFIX>_ATOMICS_H`. An item that
  clashes with one, or whose Swift name clashes with another's or with a standard library
  name the Swift uses, such as `Data`, takes a `rename`.
- Every export writes `go/<prefix>.go`. An item whose Go name clashes with another's, or
  with `Bound` or a `Tuple<N>` the Go declares, takes a `rename`.
- Kotlin returns bytes as a `Bytes`, not a `ByteArray`: Rust's buffer as `bytes.buffer`, a
  read-only `ByteBuffer`, copied up to 1 KiB and otherwise a direct one lent uncopied
  until `bytes.close()` frees it. Read it in `use { }`, or copy it out with
  `toByteArray()`. A record's bytes field is a `Bytes` too: build one to pass in with
  `Bytes(byteArray)`. Arguments stay `ByteArray`. Kotlin's objects and `Bytes` never
  closed are freed once the GC collects them, and an exported type named `bytes` clashes
  with the runtime's `<prefix>_bytes`, which takes a `rename`.
- A type parameter bounded by serde's `Serialize` or `Deserialize` takes
  `serde_json::Value`, not `String`: C passes a `<prefix>_json` it parses from JSON text
  with `<prefix>_json_parse`, not a buffer, and a generic type at such a parameter is
  named by `json` (`moq_json_snapshot_json`), not `string`. An item that clashes with
  `<prefix>_json` or its functions takes a `rename`.
- A type parameter whose bounds imply bytes, through a supertrait such as moq's
  `IntoBytes: AsRef<[u8]>` or a `From` implementation such as moq's
  `impl<B: Into<Bytes>> From<B> for Timed<Bytes>`, exports as bytes rather than
  `String`. Kotlin takes a `ByteArray` there: pass text as `"text".encodeToByteArray()`.
  C and C++ keep their types, and accept a payload that is not UTF-8 rather than aborting.
- `&[T]` of any type and `SocketAddrV4` and `SocketAddrV6` map, so a struct or enum whose
  fields were refused only for the socket addresses is a value now, not an object, and
  an item that newly exports can clash with another's C name, which takes a `rename`.
- `Vec<T>`, `PathBuf`, `SocketAddr`, and `url::Url` map, so a struct or enum whose
  fields were refused only for them is a value now, not an object: moq-tokio's
  `listen::Bind` is the C enum `moq_listen_bind`, freed with `moq_listen_bind_free(&b)`,
  not a handle. An item that clashes with the new names `<prefix>_socket_addr`,
  `<prefix>_url`, or a `<prefix>_vec_<T>` takes a `rename`.

- Each `pub` field of an object exports a setter, `<type>_set_<field>`, so a method
  named `set_<field>` on the same type, such as `Limit::set_max` beside `pub max`,
  clashes with it: give the method another name with a `rename`.
- A field is refused whole when its setter does not map, even if its getter did, such as
  one holding a reference (`pub label: &'static str`), which a handle holds `'static` and
  C's borrow for a call cannot fill: `exclude` it to leave it out.
- In `exclude` and `rename`, a path that a field shares with a method or constant of its
  type, such as `Backoff::initial`, names only the method, as in rustdoc. Name the field
  `field@Backoff::initial`: an `exclude` that left out both now needs both entries.
- Every export defines `<prefix>_contract` and `<PREFIX>_CONTRACT`, so an exported item
  that the naming rule names either, such as a crate root's `pub fn contract` or
  `pub const CONTRACT`, clashes with them: give it another name with a `rename`.
- A field, parameter, or variant named a C++ keyword, such as moq's `namespace`, takes
  a leading `_` in C as a C keyword does (`_namespace`), so the header compiles as C++.
- The C++ header's runtime names (`view`, `result`, `task`, `future`, `poll_t`, and
  `detail`) and every variant's payload name (`<enum>_<variant>`, even without fields)
  join the names the naming rule claims, so an exported item named one clashes: give it
  another name with a `rename`.
- The `instances` key is gone, top-level and under `[crates.*]`, and a config that has
  it fails to parse: delete it. A generic item exports at the instances it infers from
  its defaults, its inherent impls, its exports' implementations of the traits bounding
  it, and the signatures that name it. Exclude an instance by its path, such as
  `"Pair<u8, dep_fixture::Size>"`, to leave it out.
- An instance's C name is no longer the config's. It is the generic item's name, then
  the last segment of each argument's, with more segments only where two instances would
  share a name: `catalog::Producer<catalog::hang::Extra>` is `moq_catalog_producer_extra`,
  and `Counter::seeded` at `polar::Point` beside `Point` is
  `api_counter_seeded_polar_point`. An instance an upstream change adds can lengthen
  another's name.
- Every export writes `kotlin/<prefix>.kt`, whose runtime object `Native` makes the C
  name `native` one the naming rule claims. An item Kotlin names as a type its runtime
  uses unqualified, such as `String` or `Unit`, or as another item, such as a type
  `FooException` beside an error type `Foo`, is an error: give it another name with a
  `rename`.
