# rust2-ffi

A rustc driver that exports a crate's `pub` API as a C ABI and a C header, with no
attributes in the Rust source. It replaces [the UniFFI fork](../uniffi/README.md) for moq,
starting from C, which every other language's generator will build on.

```bash
rust2-ffi --config ffi.toml (--out dir | --census dir) [--manifest-path Cargo.toml] [-- cargo args]
```

It runs `cargo check` on the package with itself as `RUSTC_WRAPPER`, on the host, through
the front end it shares with [rust2-ts](../ts/README.md) in [`driver/`](../driver). Once
rustc has checked the configured library, it walks every `pub` item reachable from the
crate root and exports it by fixed rules, or refuses it with a compile error at its span.
It walks each other package the config lists the same way, from the metadata of the
crate the library uses through its dependency on that package, so one library holds them
all and an export names another's types across crates. `--out` writes a crate of `extern "C"` functions over the
libraries, its runtime beside it, and the header in `include/<prefix>.h`; `cargo build`
on that crate makes a static and a shared library. That crate enables each export's
features as the check did, including `--features <export>/<feature>`; such a feature on a
dependency that is no export is an error. It resolves them as the check did, wherever it
is built: its manifest carries, as a `[patch]`, each patch or `[replace]` cargo applied
to what the library builds with on any platform, read off cargo's resolution, wherever
it was declared (the manifest, cargo's config, `--config`, or the environment), keyed by
its source's URL.
A `[replace]` from the source it replaces, which no `[patch]` can carry, is an error.
Beside the C header, `include/<prefix>.hpp` holds the same API in C++, and
`kotlin/<prefix>.kt` holds it in Kotlin, over JNI functions the crate builds with
`--features kotlin`, and `kotlin/<prefix>.pro` the R8 and ProGuard rules keeping what
those functions look up by name. `python/<prefix>.py` holds it in Python, over a CPython
extension the crate builds with `--features python`.

```toml
# ffi.toml, beside the package's Cargo.toml unless --manifest-path says otherwise.
package = "moq-net"
# The prefix of every C name, the package's name in snake case by default.
prefix = "moq"
# Paths from the crate root left out: a module, an item, or one method or field. Each
# must name something exported. Nothing else leaves an item out. As in rustdoc, a path a
# method or constant shares with a field names the method, and `field@` the field.
exclude = ["track::Producer::poll_closed"]

# C names, without the prefix, for paths whose names the naming rule makes clash.
[rename]
"session::Stats" = "session_stats_snapshot"

# Another package exported into the same library, which the package builds with: its
# own `exclude` and `rename`, by paths from its crate root.
[crates.moq-pattern]
exclude = ["Pattern::new"]
```

## Rules

Every `pub` item reachable from an exported crate's root, by its shortest path, is
exported or refused. `#[doc(hidden)]` items are no part of the API and are left out; type
aliases export nothing of their own. A re-export of another export's item names that
export's, and adds nothing; another crate's type is refused.

| Rust | C |
| --- | --- |
| a struct whose fields are all `pub`, not `#[non_exhaustive]`, which map, with no `Drop` of its own | a record: a struct, copied across |
| an enum whose fields map, with no `Drop` of its own | a C enum without fields, or a struct of a tag and a union |
| any other type, if `Send` | an object: an opaque handle to an `Arc`, freed with `_free` |
| a type whose only generic parameters are lifetimes, such as `Path<'a>` | an object holding its `'static` form |
| a generic type, at each instance it infers | that instance as any other type, with each method whose impl and own bounds it meets; any other instance is refused |
| a `pub` field of an object | a getter returning a copy, and a setter assigning in place, as `x.field = value` does |
| a `pub` trait | a callback interface: a table of C functions and their data |
| a type parameter or `impl` bounded by `Fn`, `FnMut`, or `FnOnce`, which no lifetime bound or return lets outlive the call | a C function and its `void *` data, which Rust calls only during the call; any other closure is refused |
| `fn` and inherent `pub` methods | functions named through their type |
| `Result<T, E>` | `bool f(..., T *out, E *err)` |
| a returned tuple `(A, B)` | one out pointer each: `void f(..., A *out0, B *out1)` |
| a tuple inside another type, or as a field or parameter | `<prefix>_tuple_<A>_<B>`: a struct of fields `_0`, `_1`, and so on; of references only returned |
| `async fn`, or a return that implements `Future` | a future: `f` makes it, `f_poll` polls it, `f_future_free` drops it; a function that is not `async` runs at the call, as in Rust |
| `poll_x`, when `x` exports as a future | left out: the future is its poll |
| `bool`, integers but 128-bit, floats, `char` | `bool`, `<stdint.h>` types, `float`, `double`, a `uint32_t` code point |
| `&str`, `String`, `&[u8]`, `Vec<u8>`, `bytes::Bytes` | `<prefix>_buf`: a view passed in, an owned buffer returned |
| `&Path`, `PathBuf` | `<prefix>_buf` of the path's bytes as the OS holds them on Unix, which need not be UTF-8, else UTF-8 |
| `Duration` | `<prefix>_duration` |
| `SocketAddr`, `SocketAddrV4`, `SocketAddrV6` | `<prefix>_socket_addr`: its IP's bytes, port, and whether it is IPv6; one of the other family passed to `V4` or `V6` aborts |
| `url::Url` | an object `<prefix>_url`, made by `<prefix>_url_parse`, which fails with the parse error's message, and printed by `_to_string` |
| `serde_json::Value` | an object `<prefix>_json`, made by `<prefix>_json_parse` from JSON text, which fails with serde_json's message, and read back by `_to_text`; its contents are the caller's JSON library's to read |
| `Vec<T>` and `&[T]` of any other `T` | `<prefix>_vec_<T>`: an array of `T`'s C values, viewed passed in and owned returned, a slice as a clone; of references only returned |
| a returned iterator, such as `slice::Iter` or an `impl Iterator`, that no other rule maps, outside a future, made of core's and alloc's iterators over a slice, string, `Option`, `Result`, or collection, through `rev`, `chain`, `flatten`, `fuse`, `enumerate`, `peekable`, `copied`, or `cloned`, with no closure, function, or trait object inside | what `collect::<Vec<_>>()` makes of it: every item, taken at the call |
| `Waker`, taken by value | `<prefix>_waker`, as a future's poll takes it |
| `Range<T>`, `Bound<T>` of an integer | `<prefix>_range_<T>`, `<prefix>_bound_<T>`: a `<prefix>_bound` kind and a value |
| `impl RangeBounds<T>` of an integer | `<prefix>_bounds_<T>`: its start and end bounds |
| `Option<T>` | a null handle or buffer, or `<prefix>_option_<T>` |
| a `pub const` | a `#define` if a `bool` or integer, else a function returning it |
| `impl Into<X>`, `impl AsRef<X>`, or a bound implying one, as a supertrait does | `X`, or `&X` and then `String` or `Vec<T>` where `X` is `str` or `[T]` |
| `impl Into<X>`, where `X` does not map | what a `From` implementation into `X` takes: a source parameter by its bounds as above, so `impl<B: Into<Bytes>> From<B> for Timed<Bytes>` takes `Bytes`, and by each `Into<Y>` whose `Y` does not map, what converts into `Y` in turn, each `Y` once and refusing a chain longer than the crate's recursion limit; any other source as it is, so `impl<'a, V> From<&'a V> for Timed<&'a V>` takes `&V`. Two sources C passes as one buffer, such as `String` and `Vec<u8>`, are refused, unless they are forms of one value, as `&str` and `String` are |
| `impl IntoIterator<Item = X>`, where `X` is no type parameter | `Vec<X>` |
| another type parameter | `serde_json::Value` for a serde `Serialize` or `Deserialize` bound, else the first of `String`, `Bytes`, and `Vec<u8>` meeting its bounds, or a foreign implementation of its one exported trait; else an instance of the function at each type the exports implement a trait bounding it for, as `&X` where that meets its bounds too and no bound, its own or one a supertrait implies, makes it outlive the call, so C keeps its handle, and, as an argument of a generic type, at each argument of an instance that meets its bounds and makes each generic type the signature names an instance |

A generic type infers the instances it exports at, with no config: the one at its
defaults, if each type parameter has one; the one each inherent impl is of, such as
`impl Pair<u32>`; each choice, for each parameter, among its default and the types the
exports implement a trait bounding it for, one of the exports', which are `()`, a
primitive, `String`, or an exported type or an instance of one, such as
`impl Seed<u64> for Wrap<u32>`; and each instance a signature, `pub` field, or constant
names at any choice its bounds allow, whether or not the rest of it maps, which may name
more. Inference finishes before any type is mapped, so mapping never adds an instance.
An instance that holds an instance of a type inside it, such as `Cell<Cell<u8>>`, is
refused, as it could name ever deeper ones.

A record or enum holds only values and `Copy` handles: one with a field that does not map,
or that holds itself, is an object instead, and the census lists it. A borrowed return is
returned as a clone, so it must be `Clone`, and a handle in it is a new handle to the
clone. A handle outlives every call, so every lifetime an object takes is `'static`, as is
every lifetime that must outlive one; any other lifetime is the call's. A signature may
not lend C's borrow for one that is `'static`, nor return an object that borrows. A
parameter is a reference only at its top, not inside an `Option` or `Box`, and an object
no export C can call returns is refused, since C could never hold one. Everything else is
refused, and the census counts why.

## The C ABI

- **Handles** are `Arc`s, and a call borrows the value as Rust's receiver or parameter
  does, checked at run time: shared for `&T`, exclusive for `&mut T`, and moved out for
  `T`, after which the handle is only freed. A `Copy` value is copied out instead, and a
  value that is not `Sync` is shared on one thread at a time, and held exclusively by a
  future, which may be polled on any thread. A borrow Rust would
  refuse at compile time aborts, naming the function, as does a null or moved-out handle:
  there is no Rust caller to return an error to. A setter borrows as `&mut T` and takes
  its value as `T`, so it writes the value the handle holds, or the one Rust lent it,
  while a getter's copy is C's own: a nested field is got, set, and set back.
- **Arrays** of a `Vec` are a pointer, a length, and an owner, as a buffer is: passed
  in, Rust copies each element out of C's array; returned, `<prefix>_vec_<T>_free` frees
  the array and its first `len` elements, so a binding that takes the elements sets
  `len` to 0 first. A null pointer is `None`, so C passes a non-null one for `Some` of
  an empty array.
- **Values** are copied in from the caller's memory, which Rust never frees, and lowered
  into memory the caller frees once with the type's `_free`.
- **Futures** run no runtime: C polls with a `<prefix>_waker` and polls again when woken.
  A poll that finishes writes the result through the poll's own out pointers, so a
  cancelled read never loses a value. The future reuses the last waker when given the same
  one again, and owns what it borrowed: the handle it was called on stays borrowed until
  it finishes or is freed.
- **Wakers** move into Rust, which calls `drop` once it lets go of the last clone, so a
  state machine polled with a waker, such as moq's `Driver`s through kio's `Waiter`, runs
  from the caller's event loop. Rust may call `wake` and `drop` from any thread, as a
  runtime's workers do.
- **Callbacks** are a struct of a table and its data. Rust calls `free` once it drops the
  implementation, and may call the others from any thread. A callback takes scalars and
  buffers lent for the call, and returns a scalar.
- **Closures** lent for a call are a function and its data, which Rust calls only during
  the call, from any thread. One takes what a callback takes, and objects by reference,
  each lent as a handle to the value where it is for that call: C borrows it no further
  than Rust lent it, and moving out of it, freeing it, or keeping a borrow past the call
  aborts.
- **The contract** is a hash of the header's declarations, its docs included: the header
  defines it as `<PREFIX>_CONTRACT`, and `<prefix>_contract()` returns it from the library
  built with the header. A binding checks the two match before any other call, so a header
  and a library from different builds refuse to mix.
- **Panics** abort, as an `extern "C"` function's do.
- **Names** are the prefix, then the item's path from its crate's root, each segment in
  snake case, joined by `_`: `track::Producer::append_group` is
  `moq_track_producer_append_group`, and moq-pattern's `Patterns` is `moq_patterns`.
  Joining loses where segments split, and every export shares the prefix, so every C
  name is registered, and two items claiming one is a compile error at both spans, fixed
  by a `rename`: moq-net's `session::Stats` and `Session::stats` are both
  `moq_session_stats`. So are a field and the method of its name, as moq-tokio's
  `Backoff::initial`; `field@Backoff::initial` names the field. A field's setter is
  named as its getter, with `set_` after its type's name: `moq_connect_config_set_tls`,
  and `moq_backoff_set_initial_setting` once renamed.

  An instance is named by its generic item's name, then each argument but those at their
  defaults after the last that is not, so the instance at its defaults is named as the
  item. Each argument takes the last segment of its C name, such as `extra` for moq-mux's
  `catalog::hang::Extra`, so `catalog::Producer<catalog::hang::Extra>` is
  `moq_catalog_producer_extra`. Where two instances of one item would still share a name,
  each argument that tells them apart takes one more segment at a time, until they
  differ: the api fixture's `Counter::seeded` is `api_counter_seeded_point` at its
  `Point`, and `api_counter_seeded_polar_point` at `polar::Point`. The names depend on
  every instance the item exports at, so one an upstream change adds can lengthen another's
  name. A type instance's methods are named through it, and a function instance appends
  its arguments' names, a reference's by what it borrows. An `exclude` of a type or method
  by the generic type's path leaves it out of every instance, and by the instance's path
  (`"catalog::Producer<moq_mux::catalog::hang::Extra>::stats"`) out of that one. A
  `rename` matches paths as `exclude` does: by the generic item's path it renames each of
  its instances, and by an instance's path (`"catalog::Producer<moq_mux::catalog::hang::Extra>"`)
  that one alone, whose arguments then add nothing to its name, while the others keep
  theirs. The longer path wins, and at one length, the instance's.

## C++

`<prefix>.hpp` is header-only C++ over the C header, for C++17, 20, and 23, with
exceptions and RTTI on or off: it throws nothing and reads no type information.

- **Names** are the C names without the prefix, in a namespace of it: `moq_track_producer`
  is `moq::track_producer`, and its method `moq_track_producer_append_group` is
  `moq::track_producer::append_group`. A keyword takes a trailing `_` (`counter::new_`),
  as does a member named as its class or one of its fields, and a parameter named as a
  member or as anything in the namespace, which it would hide.
- **Objects** are classes owning their handle, move-only unless `Clone`, whose copy
  clones. `&self` methods are `const`, and `self` methods are `&&`, so a call that moves
  the value out reads `std::move(c).into_label()`. A handle taken by value is moved into,
  by a parameter taken by value.
- **Records** are structs, and **enums** an `enum class`, or for one with data a
  `std::variant` of a struct per variant, `<enum>_<variant>`. An enum's methods and
  traits are free functions taking it first. `PartialEq` is `==`, `Display` `to_string()`,
  `Debug` `debug()`, and `Default` `default_()`.
- **Bytes** are passed as a `view`, which any contiguous bytes convert to, and returned as
  a `buf`, which owns Rust's buffer without a copy. A `Vec<T>` is a `std::vector`, of
  `view`s for bytes passed in, and a socket address `socket_addr`, the C struct. `Option<T>` is `std::optional<T>`, a
  tuple `std::tuple`, and `Result<T, E>` a `result<T, E>`, whose `value()` aborts on an
  error.
- **Traits** are abstract classes, passed as a `std::unique_ptr` that Rust deletes.
- **Closures** lent for a call are any callable, taken by a template parameter, which
  Rust calls only during the call, from any thread. An object it lends is a reference to
  its class for that call: moving out of it aborts.
- **Futures** are move-only `future<...>`s, freed when destroyed, which cancels them and
  loses nothing. `poll(waker)` polls once; `std::move(f).get()` blocks the thread;
  `std::move(f).then(exec, callback)` polls on an executor and calls back there, returning
  a `task` that cancels when destroyed, after which the callback has finished or never
  runs and the executor is never called again; and from C++20, `co_await std::move(f).via(exec)` resumes the coroutine on the
  executor. An executor is any callable taking a `void()` job to run once, later, on any
  thread: never inline, since a wake may come from inside Rust. Each wake that lands while
  the future polls polls it again, and a future has one poller at a time, since awaiting
  it moves it.
- A **waker** is a shared callable, which Rust may call from any thread.
- **The contract** is checked when the program loads, before any other call: a header
  and a library from different builds abort.

## Kotlin

`<prefix>.kt` is Kotlin for the JVM and Android over the crate's JNI functions, which
`--features kotlin` builds into the same library, loaded by `System.loadLibrary`. It needs
kotlinx-coroutines. Each value crosses JNI as a scalar, a handle as a `Long`, bytes passed
in as a `ByteArray`, bytes returned as their owner, and anything else encoded into a
`ByteArray` as its C struct holds it.

- **Names** are in a package of the prefix: types in PascalCase without it, members in
  camelCase without their type's name (`moq.TrackProducer.appendGroup`). A keyword is
  quoted in backticks, and a member named as one Kotlin gives its type takes a trailing
  `_`. A constructor such as `new` stays a companion function: `Counter.new(1)`.
- **Objects** are classes owning their handle, `AutoCloseable`, which `close()` frees once
  no call holds it; a call after it throws. One never closed is closed once the GC
  collects it, on a daemon thread a phantom reference queue feeds. A `self` method, or passing the object by
  value, closes it once the call returns. `Clone` is `clone()`, and `PartialEq` is `equals`.
  A field is a `val`, and its setter a function, `setFirst(value)`, since setting an object
  moves it in and closes its handle, which an assignment would hide.
- **Records** are data classes, compared by their fields, and **enums** an `enum class`,
  or for one with data a sealed class of a data class per variant. `Display` is
  `toString()`, `Debug` `debug()`, and `Default` a companion's `default()`.
- **Bytes** passed in are a `ByteArray`, copied, as are those Rust lends a closure or trait
  method. Bytes Rust returns, or a record holds, are a `Bytes`, a read-only `ByteBuffer`:
  up to 1 KiB copied, and longer ones lending Rust's buffer as a direct one until `close()`
  frees it, or, never closed, until the GC collects that buffer, since a copy undercuts
  `NewDirectByteBuffer` only that far; `Bytes(ByteArray)` holds Kotlin's for a record
  passed in.
  Strings are `String`, `Option<T>` `T?`, a tuple `Pair`,
  `Triple`, or else a generated `Tuple<n>`, though a tuple of one returned whole is its element, `Duration` `kotlin.time.Duration`, and an `Err` a thrown `<E>Exception`
  holding it, whose message is its `Display`.
- A **path** is a `String`: one Rust returns that is not UTF-8 throws. A `SocketAddr`,
  `SocketAddrV4`, or `SocketAddrV6` is a `java.net.InetSocketAddress`, which must be
  resolved and, for `V4` or `V6`, of that family; one Rust returns with an IPv6 flow
  label, which Java does not keep, throws. A `Vec<T>` or `&[T]` is a `List`, and passing
  a `Vec` by value closes each object in it once the call returns.
- **Traits** are interfaces; once Rust drops an implementation, it is closed if
  `AutoCloseable`.
- **Closures** lent for a call are function types, which Rust calls only during the call,
  from any thread. An object it lends is closed once the call returns.
- **Futures** are `suspend fun`s, which resume on the coroutine's dispatcher, as C++ resumes
  on its executor, and never inside a wake: an `Unconfined` one resumes on `Default`. Cancelling the coroutine frees the future and
  loses nothing. Each call is its own future, so it has one poller. The call polls it once
  before suspending, and makes the waker it parks on only if that poll pends.
- A **waker** is a `Waker`, which Rust may call from any thread.
- **The contract** is checked when the library loads, before any other call: a Kotlin
  file and a library from different builds throw.
- **Shrinking**: `<prefix>.pro` keeps what the JNI glue finds by name, `Native`, its native
  methods, and the Kotlin functions Rust calls, while R8 strips and renames the rest. An
  Android library ships it as a consumer rule (`consumerProguardFiles`), which its AAR
  carries to each app that shrinks it.

## Python

`<prefix>.py` is Python over a CPython extension that `--features python` builds into the
same library on the stable ABI, so one build loads in CPython 3.11 and later. The module
loads the library from beside itself: `lib<prefix>.so`, `lib<prefix>.dylib`, or
`<prefix>.dll`. Its glue converts each Python value to the C value and calls the C ABI
function in-process, so a call is one crossing, which releases the GIL while Rust runs.

- **Names** are in the module: types in PascalCase without the prefix, members in
  snake_case without their type's name (`moq.TrackProducer.append_group`). A keyword, a
  member Python gives every such class, or a method named as a field takes a trailing `_`.
  Constructors such as `new` stay static methods: `Counter.new(1)`.
- **Objects** are classes owning their handle, freed once their last reference goes, or
  sooner by `close()` or a `with` block once no call holds it; a call after it raises
  `ValueError`. A `self` method, or passing the object by value, closes it once the call
  returns. `Clone` is `clone()`, and `PartialEq` is `==`, unhashable. A field is a
  property, and its setter a method, `set_first(value)`, since setting an object moves it
  in and closes it, which an assignment would hide.
- **Records** are slotted dataclasses, compared by their fields, and **enums** an `Enum`
  of upper-case members, or for one with data a class of a dataclass per variant,
  `Event.Moved(p)`. `Display` is `str()`, `Debug` `repr()`, and `Default` a static
  `default()`. Constants are module or class attributes, made anew on each read unless a
  literal.
- **Bytes** passed in are any buffer, such as `bytes`, read in place. Bytes Rust returns,
  or a record holds, are a read-only `memoryview` lending Rust's buffer until it and its
  views are released. Those Rust lends a closure or trait method are copied into `bytes`.
  Strings are `str`, `Option<T>` `T | None`, a tuple a `tuple`, `Duration`
  `datetime.timedelta`, truncated to the microsecond, with `timedelta.max` for any longer and
for `Duration::MAX`, a `Range` a `range`, and an `Err` a raised
  `<E>Exception` holding it in `error`, whose `str` is its `Display`.
- A **path** is a `str`, or any path passed in, which crosses as `os.fsencode` encodes it, so
  one that is not UTF-8 keeps its bytes. A `SocketAddr` is the `socket` module's tuple,
  `(host, port)` or `(host, port, flowinfo, scope_id)`, of that family for `V4` or `V6`.
  A `Vec<T>` or `&[T]` is a `list`, and passing a `Vec` by value closes each object in it.
  An integer out of its Rust type's range raises `OverflowError`.
- **Traits** are abstract classes an implementation subclasses; once Rust drops one, its
  `close()` is called if it has one.
- **Closures** lent for a call are any callable, which Rust calls only during the call,
  from any thread. An object it lends is closed once the call returns. An exception a
  closure or trait method raises prints its traceback and aborts, since Rust has nowhere
  to unwind it to.
- **Futures** are `async def`s, which run nothing until awaited, or for a function that is
  not `async`, an awaitable the call returns. They resume on the running event loop and
  never inside a wake: a wake from any thread queues the future's token and writes a byte
  to a socket the loop reads, without the GIL. Cancelling the task frees the future and
  loses nothing. The call polls it once before suspending, and a future is awaited once,
  as a coroutine is.
- A **waker** is a `Waker(callback)`, which Rust may wake from any thread, calling
  `callback` on its event loop.
- **The contract** is checked when the module imports, before any other call: a Python
  file and a library from different builds raise `ImportError`.

## IR

[`src/ir.rs`](src/ir.rs) is the API as each generator reads it, with no language decided:
records, enums, objects, callbacks, functions, and constants of every export, each by
its Rust path, its span, and its docs. A type is in the exact form Rust takes or returns it
(`&str` apart from `String`, `Box<dyn Trait>` apart from `impl Trait`), so a generator can
borrow where Rust borrows. [`src/abi.rs`](src/abi.rs) decides the C ABI from it once,
names and signatures, so [`src/c.rs`](src/c.rs), [`src/cpp.rs`](src/cpp.rs),
[`src/kotlin.rs`](src/kotlin.rs), [`src/python.rs`](src/python.rs), and
[`src/glue.rs`](src/glue.rs) agree; the runtime the glue includes is
[`runtime/rt.rs`](runtime/rt.rs), the one the C++ header includes
[`runtime/rt.hpp`](runtime/rt.hpp), the ones Kotlin and its JNI glue include
[`runtime/rt.kt`](runtime/rt.kt) and [`runtime/jni.rs`](runtime/jni.rs), and the ones
Python and its extension include [`runtime/rt.py`](runtime/rt.py) and
[`runtime/python.rs`](runtime/python.rs).

## Census

`--census dir` exports nothing. It writes `<package>.md` for each exported package,
which counts what exports and ranks every refusal by construct and by the type it names,
lists each refused item at its span, and ends with the `exclude` list that exports the
rest. It ignores the config's `exclude`. It still fails on an `instance not inferred`
refusal, which is a bug in the exporter's inference rather than in the crate.

## Tests

```bash
just ffi test     # export the fixtures, build them, and drive api from C, C++, Kotlin, and Python
just ffi moq      # export moq's crates at the pinned revision, build them, and run moq/moq.c, moq/moq.cpp, moq/moq.kt, and moq/moq.py
just ffi moq-cpp  # the same export, running only moq/moq.cpp
just ffi census   # census each into target/census
nix develop .#android --command just ffi android  # the Kotlin test, shrunk by R8, on the Android device adb reaches
nix develop .#bench --command just ffi bench  # frame writes and reads, through the export from C, C++, Kotlin, and Python, and moq's rs/moq-c
```

The `api` fixture covers each rule once, and names the `dep` fixture's types, which
export into the same library. `tests/api.c` runs the same script as the
fixture's `examples/transcript.rs`, and the two transcripts must match line for line;
then it breaks each runtime check, which must abort with its message. `tests/api.cpp`
runs the script through the C++ header at C++17, 20, and 23, with exceptions and RTTI off
at 17 and 23, then checks its futures: a wake that lands during a poll, a cancel between a
wake and its poll that loses nothing, and a coroutine resumed or destroyed. `tests/api.kt`
runs the script through the Kotlin on the JVM, checks its futures the same way, and breaks
each runtime check, which must throw or abort with its message. `just ffi android` runs the
same on ART: the Kotlin and `tests/api.kt`, shrunk by R8 into an app's dex with only the
generated rules keeping what JNI looks up, against the library built by the NDK for the
device's ABI, which ANDROID_HOME and ANDROID_NDK_HOME locate. Nightly runs it on an
x86_64 emulator at API 24, the oldest moq's Android library supports, and at API 35.
`tests/api.py` runs the script through the Python, in CPython's development mode with
warnings as errors, checks its futures the same way, what frees an object or bytes nobody
closed, and the values Python's types bound, and breaks each runtime check, which must raise or
abort with its message; the Python with another contract must refuse the library. The `reject`
fixture's expected errors, each with its notes, are `fixtures/reject/expected.txt`.
moq's crates export through `target/moq-ffi`, an empty library on each that `just ffi moq`
writes in place of moq-ffi's remainder. `moq/moq.c` publishes and subscribes to a moq-net
track in one process, polling each read, scopes an origin to moq-pattern's patterns and
hands it to a client to publish, polls its driver until the origin's last owner drops, reads an OpusHead into hang's catalog through moq-mux,
sets and reads a field of the catalog lent to C through `catalog::Producer::mutate`, and
serves a broadcast from a moq-tokio server bound to port 0 to a client that reads it over
QUIC, both on moq-tokio's fallback runtime, whose threads wake C's. `moq/moq.cpp` does the
same through the C++ header, short of the catalog's field, at C++17 without exceptions or
RTTI, as OBS builds it, serving on an executor's thread and blocking on each read, and
`moq/moq.kt` through the Kotlin, serving in a coroutine and suspending on each read, and
`moq/moq.py` through the Python, running the origin's driver and the server as asyncio
tasks and awaiting each read.

The C++ builds take `CXX`, or `c++`, with every warning an error and `-Wshadow`, which
stands in for MSVC's warnings that a name hides another. On Windows they take MSVC's `cl`
from a developer environment, at `/W4` with `/permissive-`, and `/std:c++latest` for 23.
They link what rustc prints a program linking the library needs. Nightly runs
`just ffi test cpp` and `just ffi moq-cpp` with Clang on Linux and macOS and with MSVC on
Windows.
