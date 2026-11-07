# rust2-ts

A rustc driver that translates a subset of Rust to TypeScript with source maps, for the browser. No WASM: the output is plain ES modules that bundlers take as is. A native runtime calls Rust through [rust2-ffi](/ffi/README.md) instead.

```bash
rust2-ts --config ts.toml (--out dir | --census file.md) [--manifest-path Cargo.toml] [-- cargo args]
```

It runs `cargo check` on the package with itself as `RUSTC_WRAPPER`, for
`wasm32-unknown-unknown`, against std checked from source with every function's MIR. The flake's
package and dev shell check it once per toolchain and pass it as `RUST2_TS_SYSROOT`; without
that, each target directory checks it with `-Zbuild-std`, which needs the toolchain's `rust-src`. Where the
package builds with getrandom or uuid, which refuse wasm32 until a binary picks their source of randomness, cargo checks it
as a dependency of a root package that picks `crypto.getRandomValues` by the feature each documents, as a browser's binary
would, in a workspace that resolves as the package's own: its lockfile, patches, and dev profile, and its paths,
relative to its root as rustc reads a member's. `--features` and `--no-default-features` pass to that dependency;
`--all-features`, a feature of the package's dependency, and a `"*"` dev-profile override, which skips workspace members
but would reach the package there, are refused. Every crate compiles as rustc
would; the configured library is also translated, after type and borrow checking, starting from its roots and following
what they reach. Every crate it depends on but std's translates with it as one program,
as far as the roots reach: each compiles with every function's MIR, and its types and
calls translate as the package's own do. They compile with `--cfg rust2_ts`, so a crate
can give translation a safe twin of its `unsafe`. Their `unsafe` translates as the
package's does, and what translation can't model is refused where it is, unless a binding
stands in for the item holding it. Only the writes of the dependencies' functions translation reaches decide what it
copies, so it runs again when it reaches one it didn't read. A construct outside the subset is a compile error with its span, and
then nothing is written. A type without a JS value is refused where a value of it is made
or received, such as a parameter, a field, or a call, rather than again at each variable
holding one; any variable or temporary that holds a type whose JS value is ambiguous, such
as `Option<Option<()>>`, is refused at its own span.

Once the output is written, `tsc` 7.0 or later, from `PATH`, type-checks it as strictly as
the runtime, but for unused names. Output it rejects is a bug in rust2-ts, and fails the
translation at the Rust span the source map names there, or else at the item's, and the
output is removed. The same run checks the bindings, and an error in translated code is
reported once they pass, since a wrong binding fails its callers too.

```toml
# ts.toml, beside the package's Cargo.toml unless --manifest-path says otherwise.
package = "moq-net"
# Paths from the crate root: a function, or a type with its non-generic methods and its
# impls of the crate's own traits, blanket ones included.
roots = ["coding::Decoder", "lite::Group"]
# Types whose `Display` and `Debug` impls come along, whichever they have, so callers can
# print them. Another crate's trait otherwise comes along only when something calls it.
printable = ["coding::DecodeError"]

# Hand-written TypeScript that stands in for an item, by its path from the crate root or
# else from a crate it depends on: a function, or a type and its functions.
[bind]
"model::flate::Encoder" = "bind/encoder.ts"
"util::TaskSet" = { file = "bind/tasks.ts", drop = true }
```

moq consumes it as the `ts` package of this repository's flake. In this repository,
`just ts run ...` runs it from source on the pinned nightly.

## Census

`--census file.md` translates nothing. It writes a Markdown report of every construct
translation refuses, with counts and spans, ranked by construct, by the type or module
they name, and by each type or path. It starts from the config's roots, as translation
does. A config that names none makes every item in the library a root, except derived
impls, which come along only when called or printable, and lowers a generic function
that no instance reaches over its own type parameters, refusing nothing that hinges on
what they are, since each instance decides that. A census succeeds whatever it finds;
translation still fails on any refusal.

`just ts census` runs it at the pinned moq revision, into `target/census`, on moq-net
whole (`moq/net.toml`), on its model's public types with their boundaries bound
(`moq/model.toml`), and on kio, and the nightly job
publishes their tables.

## Output

Each Rust module with translated items becomes a file (`lib.ts`, `coding/varint.ts`),
and each type a module of its own beside it, holding the type and its functions
(`coding/decode.Decoder.ts`), each with a `.ts.map` source map into the Rust it came
from. `rs.ts` beside them is the
runtime: what std does that JS does not, with iterators in `rs/iter.ts`, `str`'s searches
in `rs/str.ts`, the ASCII methods of `u8`, `char`, `[u8]`, and `str` in `rs/ascii.ts`, `char`'s Unicode classes, case maps, escapes, and encodings, and `str`'s case maps and escapes, in `rs/char.ts`, collections in `rs/collections.ts`, hashing in `rs/hash.ts`, ownership, atomics, and async in
`rs/task.ts`, `f64`'s and `f32`'s methods in `rs/float.ts`, `Duration` in `rs/time.ts`, and `Layout` and memory from `alloc` in `rs/alloc.ts`. What translates from std's own MIR,
`Option`'s, `Result`'s, and `Poll`'s methods, `==` of std's errors, `==` and ordering of a `Result` and a `Poll`, `cmp::max` and `min`, and the `str::parse` and
`from_str_radix` of an integer or a `NonZero` and `bool`'s `str::parse`, goes in a module under its crate's name, as
`core/option.ts` holds `map$1cf8u0y`. A dependency's modules go
under its crate's name too, `kio.ts` and `kio/lock.ts`, with the instances of its generic
functions that the package's types make, so a module at the package's root named for a
crate translation reaches is refused, and so are two versions of one crate it reaches.

| Rust | TypeScript |
| --- | --- |
| `bool`, `str`, `String`, `Box<str>` | `boolean`, `string` |
| `char` | `rs.char`: a `number` branded so, its Unicode scalar value |
| `u8`-`u32`, `i8`-`i32`, `usize`, `isize` | `rs.u8` and so on: a `number` branded with its type, checked: overflow panics |
| `u64`, `i64`, `u128`, `i128` | `rs.u64` and so on: a branded `number` while a safe integer, 32-bit limbs past 2^53, checked the same way |
| `Wrapping<T>` | the integer, whose operators wrap, and mask a shift's amount to the width, as std's do |
| `Saturating<T>` | the integer, whose operators saturate at its bounds, as std's do |
| `f64` | `rs.f64`: a `number` branded so, which is the same IEEE 754 double |
| `f32` | `rs.f32`: a `number` branded so, which a single holds exactly |
| `()`, `PhantomData<T>`, `PhantomPinned`, `..` | `undefined`, every value equal and hashing nothing; a type parameter only a `PhantomData` holds is `_T` in its struct's interface |
| `struct` | a plain object, typed by an interface; a class if it may drop |
| `enum` without data | a union of strings: `"Lite01" \| "Lite02"`, or `never` without variants |
| `enum` | a union of objects tagged by variant |
| `Option<T>` | `T \| undefined`; `T \| rs.None` where `T` can already be `undefined`, as `()` and `Option` can, so `Option<()>` and `Option<Option<u64>>` keep `None` apart from each `Some`; a generic type's `Option<T>` is an `rs.OptionOf<T>`, either as each instance's `T` decides, as a generic `Poll<T>` is an `rs.PollOf<T>` |
| `Result<T, E>` | `rs.Result<T, E>`: `{ ok: true, value }` or `{ ok: false, error }`; `rs.OK` or `rs.ERR` where the payload is `undefined`, as in `Ok(())` |
| `ControlFlow<B, C>` | `rs.Result<C, B>`, `Ok` to continue; what `?` branches on is the value it was applied to, but for a `Poll`'s `Pending` and `Ready(None)`, which continue as `Ok` of themselves |
| tuple, `[T; N]`, `&[T; N]` | an array, and `rs.Slice<T>` of all of it for `&[T; N]`; for bytes, `u8` or `MaybeUninit<u8>`, `Uint8Array` for `[u8; N]`, and `rs.ByteSlice` for `&[u8; N]` |
| `&[T]`, `&mut [T]`, `Box<[T]>` | `rs.Slice<T>`, a range of an array; `rs.ByteSlice` for bytes |
| `Vec<T>` | `rs.Vec<T>`: its items, the array `items`, and the capacity std's holds; `rs.vecFrom(items)` makes one of an array with room for exactly them. `rs.ByteVec` of bytes |
| `Cow<[T]>`, `Cow<str>` | `rs.Cow<T>`: the `Vec` it owns, or an `rs.Borrowed` holding the slice it borrows; `rs.ByteCow` for bytes, an `rs.ByteVec` or an `rs.ByteSlice`; `rs.StrCow` for text, the string it owns or an `rs.Borrowed` holding the one it borrows, which `rs.strCowStr` reads |
| `&T`, `&mut T`, `Box<T>`, `Reverse<T>`, `NonZero<T>`, `AssertUnwindSafe<T>` | the value; a `&mut` to a value JS copies, such as a number, an `Option`, a `String`, or a reference, or to an `Arc`, a `Waker`, or a `LocalKey`, whose copies share what it points at, is an `rs.Place<T>`, which for what a box holds is the place holding the box |
| `*const T`, `*mut T`, `NonNull<T>` | what a `&mut T` is: the object, an `rs.Slice` of a `[T; N]`'s elements or an `rs.ByteSlice` of a `[u8; N]`'s bytes, which compares by the array or buffer and offset they are at, refused for an array of one, which `array::from_ref`, `from_mut` of a variable, and `Option::as_slice` make of their own, or an `rs.Place<T>` to a value JS copies and to a library type such as a `Vec` or an `Arc`, which into an array, slice, or `Vec` is the `rs.AtIndex` of its element, which `add`, `sub`, `offset`, their wrapping forms, `offset_from`, `ptr::copy`, and `slice::from_raw_parts` move, measure, copy between, and view; one to a value in no array is element 0 of an array of one, as in Rust; JS keeps that array, and a shared subslice, which JS copies, apart from what holds it in Rust, so a pointer, slice, or copy the body shows reaching past either, a raw or mutable slice of a byte in no array, and `offset_from` between pointers the body shows JS keeps in different arrays are refused, and through a pointer from elsewhere, such as a field, any of these, or a read or write past the array JS keeps it in, fails at run time; a pointer into an array of values JS keeps as objects is refused, as is comparing pointers to other values JS copies, each a place JS makes of its own; `rs.NULL` for null and `rs.DANGLING` for dangling. One made from a reference, by `&raw const *r`, `as`, `ptr::from_ref` or `from_mut`, or `NonNull::from`, is where the body shows the reference borrowed, as the `rs.AtIndex` of the element `&v[i]` lends, and else a place of its own: holding the value of a shared reference, which nothing may write while the pointer lives, or writing a whole value into the object a `&mut` is in place. A `*const ()` is an identity, such as a waker's data, and `rs.DANGLING_UNIT` when dangling; one to a place of `()` or from a reference to one is refused. A pointer to a value JS keeps as an object cast to one, by `as` or `cast`, is the same object, which `rs.ptrErase` notes the type of, so a cast back to a pointer to another type fails at run time, and null and dangling stay so, dangling at the alignment of the type it was cast from; one to a value JS copies is the same place where an `Arc`'s or `Rc`'s `into_raw` or `as_ptr` gave it, its value's one place, which `rs.arcErase` checks where the body doesn't show it, and refused where the body shows a borrow's, a new place each borrow, as is casting one to a view of bytes, a new one each borrow, or one of no fixed size. A write of a whole object copies into it one object deep, as an assignment through a `&mut` does, the state an `rs` class such as `rs.LazyCell` keeps private included, so a pointer to a part kept as an object, such as a field of a tuple or struct type, is refused wherever the crate may put another object there while a pointer into that memory may still be read, which a write into a fresh allocation, or a variable before any pointer into it is taken, can't, whether by index or through std's iterators over its slots; a read copies what the crate writes into in place |
| `MaybeUninit<T>` | `T`: `undefined` until written, `0` for a byte, or an object whose parts kept as objects are there, holding `undefined`, as each of `Box::new_uninit_slice`'s is; a slice's `write_clone_of_slice` clones into its slots without dropping what they held |
| `ManuallyDrop<T>` | `T`, which nothing drops but its own `drop`; its `Debug`, comparisons, and constants are refused |
| `union` | an object of slots named by offset, `at0` and `at8`, each holding the field part kept there; fields must overlap in the same type, or in a non-null pointer and an `Option` of it |
| an iterator | `rs.Iter<Item>`, whose adaptors are runtime functions: `rs.iterMap(rs.iterSlice(v), f)` |
| `a..b`, `a..=b` | `rs.Range<T>`, `rs.RangeInclusive<T>`, which are iterators too |
| `a..`, `..b`, `..=b`, `Bound<T>` | `{ start }`, `{ end }`, `{ end, inclusive: true }`; `rs.RangeBound<T>`, a union tagged by variant |
| a range a drain or a B-tree's `range` or `extract_if` takes | itself, or of a type other than std's ranges and a pair of `Bound`s, `rs.boundsOf` of it and its `start_bound` and `end_bound`, which the runtime calls where std's does; one that drops, the parameter of the call `rs.dropAfter` makes, which drops it once the call returns, as std drops it before it does anything JS can see, or of an `extract_if`, which holds it, the parameter of a call that hands its iterator what drops it after its closure |
| `HashMap`, `HashSet`, `BTreeMap`, `BTreeSet`, `VecDeque`, `LinkedList`, `BinaryHeap` | `rs.HashMap<K, V>` and so on, each a plain object whose functions are the runtime's: `rs.hashGet(map, key)` |
| `PeekMut<T>` | `rs.PeekMut<T>`, the `rs.Place` of a heap's greatest element, which sifts it down as it drops once `DerefMut` lent it |
| `Ordering`, `DefaultHasher`, `RandomState`, `BuildHasherDefault<H>` | `rs.Ordering`, which is `-1`, `0`, or `1`; `rs.Hasher`; `rs.RandomState`, the keys of the `DefaultHasher` it builds, never changed once made; `undefined`, as `PhantomData` is, whose `build_hasher` is `H::default()` |
| `Duration` | `rs.Duration`: `{ secs, nanos }`, std's whole seconds as an `rs.u64` and the nanoseconds past them, never changed once made |
| `Layout`, `alloc::alloc` | `rs.Layout`: `{ size, align }`, never changed once made, checked as std's constructors check it; `alloc` gives an `rs.Block` of its bytes, which a cast of its pointer to `*mut T` the body shows came from `alloc` or `realloc` makes a JS array of `T`s, values JS copies, as an `rs.AtIndex` of its first; a cast back to `*mut u8` only as what `dealloc` or `realloc` takes, which free the array or copy it into a new one; its bytes, read, written, or cast to any other type, are refused, and fail at run time through a pointer from elsewhere |
| closure | an arrow function, with `[Symbol.dispose]` if it owns a value that drops, and what makes its clone if a clone can't be itself |
| `fn` pointer, function item as a value | the function, or std's an arrow that calls it as a written call does, as a closure is typed: a `()` result is `undefined`, where a declared function returns `void` |
| `&dyn Trait`, `&mut dyn Trait`, `Box<dyn Trait>`, `Arc<dyn Trait>`, `Rc<dyn Trait>`, `Weak<dyn Trait>`, `Pin<P>` of one | an object with the trait's methods, its supertraits' included: `shape.area$Shape()`; behind an `Arc` or an `Rc`, an `rs.ArcDyn` sharing the count of the `Arc` it was made from |
| a pointer to a struct whose last field unsizes to a trait object: `Rc<RefCell<dyn Trait>>`, `Arc<Mutex<dyn Trait>>`, `Arc<RwLock<dyn Trait>>`, `&mut Cell<dyn Trait>`, `Rc<UnsafeCell<dyn Trait>>`, `Arc<Mutex<dyn Future + Unpin>>`, the crate's own `&Tail<dyn Trait>` | `rs.tailDyn` of the struct it was made from: the same struct, whose other fields, borrows, and lock are that struct's own, and whose last field is the trait object a `&mut` to that field would be, made where it is read; a `dyn Future`'s polls the one future in place and drops it with the struct. A `ManuallyDrop<dyn Trait>` is the trait object of what it holds, which a box of it never drops |
| `dyn Error` | an object of its `fmt$Debug` and `fmt$Display`, and its `source` where a call dispatches to it: `e.source$Error()` |
| `dyn Any` | an instance of the class its value's type was wrapped in, which a downcast checks for: `a instanceof of$dyns_Square ? a.v : undefined` |
| `dyn Fn`, `dyn FnMut`, `dyn FnOnce` | the closure or function it was made of: `(a0: rs.u8) => rs.u32` |
| `dyn Iterator`, `dyn DoubleEndedIterator`, `dyn ExactSizeIterator`, `dyn FusedIterator` | `rs.Iter<Item>`: `rs.iterDyn` of the iterator it was made of |
| constant | its value, inlined as Rust inlines it, so each use of one holding a cell is a cell of its own; one with no value tree, or holding a cell, read from the memory rustc evaluated |
| `static` | a module's value, named for the static, that every use shares, read from the memory rustc evaluated; one that reads another static or a class is an `rs.LazyCell` of it, made on first use, since that may not be made yet when its module is. A `static mut`, and a pointer in one into memory the translation can't name, fail translation |
| a type that drops | a class with `[Symbol.dispose]`: its `Drop` impl, then its fields |
| a type's function | a function of the type's module, taking `self` first: `Decoder.slice(dec, 4)` after `import * as Decoder` |
| `Arc<T>`, `Rc<T>`, `Weak<T>`, `Mutex<T>` | `rs.Arc` for either, of what a `Box<T>` is for a slice or `str`, with its strong and weak counts, an `Arc<[T; N]>` unsized to an `Arc<[T]>` an `rs.ArcDyn` of the slice of its array, sharing its count, an `Arc<[u8]>` of an `Arc<str>`'s bytes sharing its count, and `new_cyclic`'s `Weak` upgrading only once its value is made, `get_mut` lending its value only while no other `Arc` or `Weak` shares it, `make_mut` cloning it into a new `rs.Arc` while another `Arc` shares it, or moving it there while only `Weak`s do, which then dangle, as a copy where pointers to it compare, `as_ptr` and `into_raw` pointing where a reference to its value does, or at its place where JS copies it, as a pointer the body shows made from a reference to that value does, which `from_raw`, `increment_strong_count`, and `decrement_strong_count` take back to its `rs.Arc` and count, refused for one the body shows a borrow made, and failing at run time on another pointer no `as_ptr` or `into_raw` of that type of `Arc` gave, or one whose value dropped, and refused for one to a slice or bytes, which JS makes anew each time, and `ptr::addr_eq` comparing what `ptr::eq` compares, of one pointee type, and `into_inner` giving it only from the last strong `Arc`, after which its `Weak`s no longer upgrade; `rs.Weak`, counted in its `rs.Arc` until it drops, and of no `Arc` once made by `Weak::new`; `rs.Mutex`, its own guard, which a lock gives as `Ok` and the guard's drop unlocks: locking it again while held panics and `try_lock` gives `WouldBlock`, as wasm32's do, and a guard's `map` and `filter_map` make `rs.MutexMap`s, each where the part it lends is, which hold the mutex locked until they drop, and a panic aborts, so nothing poisons it and `thread::panicking` is `false`; a `Mutex`'s, a `RefCell`'s, and an `RwLock`'s `into_inner` gives its value and `get_mut` lends it, or an `rs.Place` of it where JS copies it |
| `PoisonError<T>`, `TryLockError<T>` | the `T` it holds; `rs.TryLockError<T>`, tagged as `Bound` is |
| `Cell<T>`, `UnsafeCell<T>`, `RefCell<T>`, `RwLock<T>` | `rs.Cell`, the place holding its value in `v`, for either of the first two: `UnsafeCell::get`'s and `Cell::as_ptr`'s pointer, and `get_mut`'s reference, is the cell where it is an `rs.Place`, and otherwise the object in `v`, refused where the crate may put another cell or object in its place; `rs.RefCell` for either of the others, its own guard, counting its borrows: borrowing it while that conflicts panics as `RefCell`'s does, and locking it so aborts as wasm32's `RwLock` does; `try_borrow` and `try_borrow_mut` fail with `rs.BorrowError` and `rs.BorrowMutError` instead, a clone borrows the cell, `Ref::clone` counts another borrow of it, and a guard's `map`, `filter_map`, `try_map`, and `map_split`, an `RwLock` guard's included, make `rs.RefMap`s, each where a part it lends is, which a write through a mapped `RefMut` reaches in place, sharing the cell's borrow until the last drops; `RwLockWriteGuard::downgrade` keeps the lock as one read, and `Ref::leak` and `RefMut::leak` keep the borrow until `RefCell::undo_leak` ends it; `Cell::update` calls its function on a copy, and `RefCell::replace_with` on the value, borrowed as `borrow_mut` does, or its place where JS copies it; refused for a part of a sized type JS can't write in place, such as a binding's |
| `thread_local!`, `LocalKey<T>` | an `rs.LocalKey`, a module's value that each copy of the key is, which makes its value on first use from the `const` or the expression and never drops it, as wasm32's do; `with` calls its function on that value, `try_with` never fails, the `Cell` and `RefCell` helpers lower from std, and a `RefCell` key's `set` and `with_borrow_mut` borrow it as `borrow_mut` does |
| `OnceCell<T>`, `OnceLock<T>` | `rs.OnceCell` and `rs.OnceLock`, holding the `Option<T>` in `v`, which is the place `get_mut` lends where JS copies the value; a `OnceLock` initialized from within its own initialization panics, as wasm32's does, and `get_or_try_init`'s error leaves either empty |
| `Once`, `OnceState` | `rs.Once`, an `rs.OnceLock` that holds `true` once `call_once` or `call_once_force` ran its function, and panics as wasm32's does if run again from within it; `rs.OnceState`, that `Once`, which a panic never poisons, as it aborts |
| `LazyCell<T>`, `LazyLock<T>` | `rs.LazyCell` and `rs.LazyLock`, which run their function on the first `force` or `Deref`, once, and drop it then, or with the cell if it never ran; forced from within it, either panics as std's does |
| `AtomicU32` and the others of `bool` and integers, `AtomicPtr<T>` | `rs.Atomic<T>`, the `rs.Place` of its value; an `atomic::Ordering` is its discriminant, and one an operation can't take panics as std's does. An `AtomicPtr` compares pointers only to what JS keeps as an object or bytes, as raw pointers do. A spin-wait's `hint::spin_loop` and `thread::yield_now` do nothing, as neither promises anything observable |
| `async fn` | a function returning `rs.Future<T>`: a class of its own, its coroutine's state machine |
| async closure | a closure returning the future of each call, with what makes the future of a call that consumes it if it owns a value that drops and `Fn` or `FnMut` call it |
| `Poll<T>`, `Waker`, `Context`, `RawWaker`, `RawWakerVTable` | `T \| typeof rs.Pending`; `T \| rs.OuterPending` where `T` can already be pending, as `Poll<Poll<u8>>` can, so `Ready(Pending)` keeps apart from `Pending`; `rs.Waker`, which is also the context, and whose `data` and `vtable` are its identity, which `Waker::data`, `Waker::vtable`, and `will_wake` read, and `clone_from` keeps where the waker it has `will_wake` the source; `rs.RawWaker` and `rs.WakerVTable`, records of their fields under std's names, compared by their derived `==` |
| `pin!(x)`, `poll_fn(f)`, `future::pending()`, `future::ready(v)` | `x`; `rs.PollFn`, which drops `f` with itself; `rs.NEVER`, which every `Pending` is; `rs.Ready`, whose first poll or `into_inner` takes `v` and a later one panics, and which drops `v` with itself unless one took it |
| `Box::pin(x)`, `dyn Future<Output = T>` | `x`; `rs.Future<T>`: an `async` body or std's future as it is, and another future as an `rs.DynFuture` over its `poll`, which drops it with itself unless borrowed |
| `bytes::Bytes` | `rs.ByteSlice`, as `&[u8]` is: its clones and slices share bytes that never change |
| `bytes::BytesMut` | `rs.BytesMut`: a window of storage, which what it splits off and freezes share without overlapping |
| `rand::rngs::ThreadRng` | `rs.ThreadRng`, which holds no state |
| `hashbrown::HashMap`, `HashSet` | `rs.HashMap<K, V>` and `rs.HashSet<K>`, as std's, which are hashbrown's |
| `hashbrown::HashTable<T>` | `rs.HashTable<T>`: an `rs.HashMap` of nothing to each item, under the hash its caller gives; its entries are `rs.TableEntry<T>`, the place of the item an occupied one holds |

`Vec<u8>` storage and `[x; N]` byte arrays up to 1 KiB share 8 KiB `ArrayBuffer`s, as Node's
`Buffer` pool does, so the `buffer` behind a view holds other bytes too: read it through
`byteOffset` and `byteLength`, and copy it before transferring it.

An `rs.ByteSlice` is a range of a `Uint8Array`: its `data`, from `start`, for `length`
bytes. A struct's private `&[u8]` or `&[u8; N]` field is its three parts, `bufData`,
`bufStart`, and `bufLen` for `buf`, so a cursor moving through it makes no object; a
read that needs the slice whole makes it there. A caller passes bytes as
`rs.byteSliceFrom(array)` and reads a slice it gets as `rs.byteSliceView(slice)`, a view
of it. An `rs.Slice` is the same of other values, a `&[T]`, `&mut [T]`, or `Box<[T]>`:
its `data`, a JS array, from `start`, for `length` elements, read and written where they
are, so `&mut v[a..b]`, `split_at_mut`, `chunks_mut`, and `split_mut` lend parts of the
same array. A caller passes one as `rs.sliceFrom(array)` and reads one it gets as
`rs.sliceItems(slice)`, the array itself where the slice is all of it. A `[T; N]` stays a
JS array, and a `&[T; N]` or `&mut [T; N]` is an `rs.Slice` of its elements where they
are, as a `&[u8; N]` is an `rs.ByteSlice`, so `first_chunk_mut`, `split_first_chunk_mut`,
a subslice pattern, and `array::from_mut` lend part of the same array, and a whole write
through one writes each element in place. A slice of arrays has each as a JS array of its
own, so `as_chunks_mut` and `as_rchunks_mut` lend their runs as a `Proxy` of an array
whose elements are views of each run, a `Uint8Array` one for bytes, and a whole write of
a run copies into it.
Before that write, a view of other values that a read took detaches to a copy of what it
held. A byte run's view can't, so it is copied wherever it is read by value: the
call counts as a write into the bytes, which has translated code copy each `[u8; N]` it
reads, and the runtime's swaps, rotations, and sorts move copies of the runs.

`usize` is 32 bits, as on wasm32, so it is exact in a `number`. Arithmetic is checked as
a debug build checks it, so translated code panics where native code would: it never
wraps silently. A panic throws `rs.Panic`, with the message native Rust panics with, and
aborts as wasm32's does: it runs no cleanup, so what the translation held is undefined
after one, as a wasm trap leaves it, and a caller that catches `rs.Panic` must not use it
again. An abort without a panic, as std's for an `RwLock` locked again while held,
throws `rs.Abort`, a `Panic` with std's abort message. So `catch_unwind` only ever returns
`Ok`, `resume_unwind` throws `rs.Abort` without running the hook, and `panic_any` panics
with its payload if that is text, or `Box<dyn Any>` as std's hook writes any other.
A program that reaches `set_hook` has each panic carry the `Location` native code gives
it, std's own at the `/rustc/<commit>/library/...` path its build names, read from the
toolchain's source, and runs its hook with a `PanicHookInfo` before throwing; a panic in
the hook throws `rs.Abort`, as std aborts. A panic of `bytes`' own code is where the
program's `bytes` has it, named as the session names that file: by the host's cargo
registry, unless the build remaps it, as `--remap-path-prefix` does natively. The default
hook writes nothing, as wasm32 has no stderr. A program without one passes no locations,
and a bundler drops the hook's runtime, `rs/hook.ts`, from it; what stays is the location
parameter of the runtime's panicking functions, which it leaves unset, and the key each of
std's and `bytes`' panics passes.

`Location::caller` gives rustc's location: the file as the session names it, and line
and column. A function rustc passes its caller's location to, such as a `#[track_caller]`
one, takes it as an `rs.Location` last parameter, and so does a trait object's method
whose trait declares it so. A call the crate writes passes the call's location, or, in such
a function, its own caller's; a function pointer, or a trait object's method only the impl
tracks, passes the function's own, as rustc's shim does. One that std's code calls,
through its `Fn` impls, is refused, as its location is in std's source.

Each integer type is its own type to `tsc`, named as Rust's: `rs.u8` is a `number` with a
type-only brand, so a `u16` where a `u8` goes fails the type check, while a plain
number, such as a literal, passes for any. A cast that keeps the value is `x as rs.u16`,
which costs nothing at run time.

A wider integer is a `number` up to 2^53 - 1 in magnitude, and past that an immutable
object of 32-bit limbs, `{ hi, lo }`, with `w2` and `w1` between them at 128 bits and
`hi` signed in a signed type. Each value has one form, so `x === 0` is exact; a caller
makes one past 2^53 with `rs.u64Of(18446744073709551615n)`. Its other operators and
comparisons are runtime calls, `rs.u64Add(a, b)` and `rs.u64Lt(a, b)`, and `x as u32`
is `rs.u64Low(x)`. `((hi as u64) << 32) | lo as u64` of halves of at most 32 bits is
one call, `rs.u64FromHalves(hi, lo)`.

Each integer's inherent methods are std's, and a `NonZero`'s, a `Wrapping`'s, and a
`Saturating`'s are its integer's, as std defines them: a `Wrapping`'s `pow` is
`wrapping_pow`. `pow` and `abs` panic on overflow as a debug build's do. Those the operators
lack are one runtime function for every width, `rs.intCheckedPow(a, n, 64, false)`, given
the width and sign, which computes on the value's `bigint`.

The stable ASCII methods of `u8` and `char`, and of `[u8]` and `str`, so of a `Vec<u8>`
and a `String`, are std's. A `u8` and a `char` share each runtime function, `rs.isAsciiDigit(c)`.
`make_ascii_uppercase` and `make_ascii_lowercase` of a byte, a char, or a string write
the other case where the `&mut` borrows it, a part of a `String` included. `escape_ascii`'s iterators read from either
end, size themselves as std's do, and write what they have left by `Display`, unpadded.

`char`'s Unicode classes and case maps read std's Unicode tables, which the runtime
carries in `rs/unicode.ts`, not the engine's, so they hold of the chars std's do whatever
the engine's Unicode version. `to_lowercase`, `to_uppercase`, and the escapes read from
either end, size themselves as std's do, and write what they have left by `Display`,
unpadded, as `escape_debug` of a char it leaves as it is writes that char even once it has
yielded it. `encode_utf8` lends the `&mut str` of the bytes it wrote, which a write through
it changes in place, and `encode_utf16` a view of its units. `char::decode_utf16`,
`char::from_digit`, `char::try_from` a `u32`, and `u8`'s and `u16`'s `try_from` a `char` are
std's, their errors and panics included.

`str`'s `to_lowercase` and `to_uppercase` map each char by the same tables, but for a
capital sigma, which `to_lowercase` writes final at the end of a word, as std reads its
cased and case-ignorable neighbors. `escape_debug`, `escape_default`, and `escape_unicode`
are std's `flat_map` of each char's, sized as it is, and write what they have left by
`Display`, unpadded. `encode_utf16` is sized as std's, and `Debug` of it is `EncodeUtf16 { .. }`.
`String::from_utf16` and `from_utf16_lossy`, and `from_utf16le`, `from_utf16be`, and
their lossy forms over bytes, decode as `char::decode_utf16` does: an unpaired surrogate
fails with `rs.FromUtf16Error`, `{ kind }`, its private `FromUtf16ErrorKind`'s name, or
becomes U+FFFD, and so does an odd byte at the end.
`String::from_utf8_lossy` is a `TextDecoder`'s, which writes one U+FFFD for each maximal
invalid sequence, as std's does, and borrows the text of valid UTF-8, as std's does. `utf8_chunks` splits
where `str::from_utf8` fails, so each `Utf8Chunk`'s `invalid` bytes are std's, and `Debug`
of it writes what is left as std's does, each invalid byte as `\xNN`. `from_utf8_unchecked`,
of a `str` or a `String`, decodes as `from_utf8_lossy` does the UTF-8 its contract promises.
`String::into_bytes` and `Vec::<u8>::from` a string encode its UTF-8 anew.

An `f64` is a JS number, so its operators are JS's, which round as Rust's do. Where
JS differs from std, the runtime has std's: `round` halves away from zero, `max` and
`min` ignore a NaN, and a cast to an integer truncates, saturates at the type's bounds,
and sends NaN to 0. A 64- or 128-bit integer `as f64` rounds once, to the nearest. The
functions std leaves to the platform's libm, such as `sin`, `exp`, `ln`, `powf`, and
`hypot`, are `Math`'s, which the tests find within 2 ULPs of native Rust's in Node.
`mul_add` rounds once, as wasm32's software `fma` does: Dekker's exact product and
Boldo and Melquiond's sum rounded to odd, or the operands' exact `BigInt` sum where the
product's error would be subnormal or a step would overflow. It costs about 12 ns a call
in Node where `a * b + c` costs 1, and microseconds on the `BigInt` path. `clamp` panics
with std's message. `Duration`'s `f64` methods run std's integer algorithm, to the
nearest nanosecond with its errors and panics. JS doesn't say which NaN an operation makes, so a NaN's payload and sign are the engine's: Node and Chrome
keep them through `from_bits` and `to_bits`, but V8 quiets a signaling NaN stored in an
array, as a `Vec<f64>` holds it.

An `f32` is a JS number a single holds exactly. Each arithmetic result, and each of
`sqrt`, `recip`, and libm's functions, rounds to a single with `Math.fround`, which gives
the single nearest the exact value, since a double holds the exact result closely enough;
`%`, comparisons, and negation need no rounding. Its methods compute in singles where std
does, as `div_euclid`, `powi`, and `atanh` do, and in doubles where std does, as wasm32's
`midpoint` does. `mul_add` adds the exact double product rounded to odd, which
`Math.fround` then rounds once. A cast to `f64` is the same number, and `f64 as f32` is `Math.fround`;
an integer rounds to a single once, never through the nearest double, which could round
twice. A NaN keeps its sign and payload at the top of a double NaN's, so `from_bits` and
`to_bits` keep a signaling one, which JS's conversion from a single would quiet, and V8
quiets one stored in an array as it does an `f64`'s.
`Duration`'s `f32` methods are std's: `as_secs_f32` and `div_duration_f32` round each step
to a single, and the rest go through `f64` as std's do.

`format!`, `write!`, `panic!`, `assert!`, and the other macros of `format_args!`
translate, as do `Display` and `Debug` impls, derived and thiserror's included. The
runtime ports `core::fmt`, so the text matches native Rust's byte for byte. A placeholder
becomes a function writing its value, and `fmt::Arguments` an array of literal pieces
and such functions: `rs.fmtWriteFmt(f, ["code=", (f) => rs.u16Display(code, f)])`. A `&dyn
Debug` or `&dyn Display` is such a function too, and so is a `&mut` one, which reads
through the `rs.Place` of a value JS copies as it writes, a `Box` of one, `rs.fmtOwned`
of the value, which drops it as the box drops, and an `Arc` or `Rc` of one, which the last
pointer drops, as an `Arc` of the crate's own trait object does. Format specs take every option but `{:p}`:
fill, alignment, `+`, `#`, `0`, `x?` and `X?`, a width and a precision, each also from an
argument, and `x`, `X`, `o`, `b`, `e`, and `E` for integers, `e` and `E` for floats. An `f64`
prints std's digits: the shortest that read back as it, the greater of two as close where
JS's would take the even one, or its exact value rounded half to even to a precision,
where `toFixed` rounds half up. An `f32` prints the shortest digits that read back as the
single, found as Dragon does, which JS has no function for. `str::parse` of a float takes
std's grammar, not `Number`'s, and rounds to the nearest of the type once, so an `f32`
never rounds through a double. The `str::parse` and `from_str_radix` of an integer or a
`NonZero` are core's own, over the string's UTF-8 bytes; their `ParseIntError` is
`rs.ParseIntError`, `{ kind }`, whose `IntErrorKind` is the variant's name. `bool`'s is
core's too, failing with an `rs.ParseBoolError`; a `char`'s is `rs.charFromStr`, failing with
an `rs.ParseCharError`, `{ kind }`, its private `CharErrorKind`'s name. `==` of a `Result`,
a `Poll`, and std's errors is their derived `PartialEq`, from core's MIR, so two errors are equal
where their kinds are, private or not. A `Result`'s and a `Poll`'s `PartialOrd` and `Ord` are
their derived ones too: by variant, `Ok` before `Err` and `Ready` before `Pending`, then by
what each holds. `Debug` of a string or `char` escapes by
std's Unicode tables, which the runtime carries, not the engine's. `Debug` of std's
collections writes their items in the order each iterates, a hash map's or set's included,
and of their iterators and `Chars` the items each has left, in std's wrapper; a deque's
`Iter` and `IterMut` write the two slices of its buffer they have left, swapped as std's
swap them, and its `Drain` its counts; a list's `Iter` and `IterMut` write what is left and
its length, as std's do. `Debug` of std's iterator adaptors, `Map` through `Flatten`,
`DecodeUtf16`, and a `str`'s escapes, writes their fields as std's does, each iterator it
wraps by its own `Debug`, no closure, and the state each keeps, such as what a `Peekable` peeked, what a
`Skip` has left to skip, and the countdown std's `StepBy` of a small unsigned range keeps
in its end. So does `Debug` of std's sources, `Once`, `OnceWith`, `FromFn`, `Repeat`,
`RepeatN`, `RepeatWith`, and `Successors`, an `Option`'s, a `Result`'s, and an array's
iterators, a `CharIndices`, a `String`'s `Drain`, and a slice's `Windows`, chunks, and
splits, each writing what is left of its slice as std's does, emptied where std's `nth` past
the end empties a `Windows`; a hash set's operations write the list a clone yields, and a
`BTreeSet`'s the way std picked to make each, by the iterators it reads, the set it looks
keys up in, and the key it peeked or holds back. A list's and a B-tree's `ExtractIf` write
the item or entry it reads next, until it stops, and a hash map's and set's nothing. A
`str`'s searches, `Split` and its forms, `Matches`, `MatchIndices`, `Lines`,
`SplitWhitespace`, and `SplitAsciiWhitespace`, write their searcher's state in UTF-8 bytes
as std's does, a `&str`'s two-way search replayed to find what it last compared, which
plain searches never keep. A slice's `chunks_mut` and its forms are refused, as std writes
their slice's address. `Debug` of `Bound`, `Ordering`, `Reverse`, `AssertUnwindSafe`, and `ControlFlow` is
their derived one, whichever way each is held, and a guard of a cell or a lock, mapped or
not, writes what it lends by its own `Debug` and `Display`. `Debug` of a `Cell`, a
`RefCell`, a `Mutex`, and an `RwLock` writes what each holds, or `<borrowed>` and
`<locked>` while a guard keeps it from being read, as std's do; an `UnsafeCell` writes only
its name. A `LazyCell` and a `LazyLock` write their value, or `<uninit>`, unpadded, before
it is forced and while their function runs, without forcing it. A caller prints a
`printable` type as `to_string()` and `format!("{err:?}")`
would: `rs.fmtToString((f) => DecodeError.fmt$fmt_Display(err, f))` and
`rs.fmtFormat([(f) => DecodeError.fmt$fmt_Debug(err, f)])`, after
`import * as DecodeError from "./coding/decode.DecodeError.ts"`. `fmt::Write` translates for a
`String`, a `Formatter`, and the crate's own writers, through a `&mut` to any of them, and
through a `dyn fmt::Write` of any of them.
A crate's own provided `write_char` and a literal `write_fmt` call its `write_str` once, as
std's do. What placeholders write reaches its `write_str` and `write_char` in the calls std
makes, with the same text in the same order: each fill a `write_char`, an integer's sign one
before its digits, a float's formatted parts, and a string's `Debug` in runs between its
escapes, so a writer that counts calls or fails partway sees what it would natively.

An iterator's `next()` returns its next item or `rs.DONE`, never `undefined`, so an
iterator of `Option` keeps each `None` apart from its end. std's adaptors and consumers
are runtime functions taking the iterator first, `rs.iterMap(it, f)`, lazy and
short-circuiting as std's are, and call their closures, and read what they wrap, in
the same order and as often, a skip by `nth` included. `max`, `min`, and their `_by_key`
forms test each next item's key for `<` the best's, as std's do, `max_by` and `min_by`
compare the best and then the next, and a B-tree's iterators read their ends. `eq`,
`cmp`, `partial_cmp`, the tests on it, and their `_by` forms read the first side by
`try_fold` and the other by `next`, as std's `iter_compare` does, and `eq` of two
`TrustedLen` iterators whose size hints' upper bounds differ reads neither.
An iterator over a slice, array, or `Vec`, or a small range, that only its `next` and
moves into other variables read steps in place, not one moved into a field, as an `async`
block's is when held across an `.await`, as two variables: the array and an index into it, or the
range's bounds, with no `rs.Iter` made. The crate's own `impl Iterator` returns `rs.DONE` from its `next` too, and joins
std's adaptors through `rs.iterFrom`, which calls its own `size_hint`, `next_back`, `len`, `nth`, and `nth_back`,
and of one it owns, not borrows by `&mut` or boxes, its own `fold`, `rfold`, `count`, `last`,
`find`, `rfind`, `find_map`, `for_each`, `reduce`, `all`, and `is_sorted_by`, where std's
adaptors and consumers call them: a filter's `next` calls `find`, an `extend` into a hash or
B-tree collection, a `String`, or a tuple calls `for_each`, `max` calls `reduce`, and
`is_sorted` calls `is_sorted_by`, which calls `all`. One whose JS value is copied, such as an
enum's, it holds as an `rs.Place` of its own, which its `next` and std's methods taking
`&mut self` write through, as they do the place a `&mut` to it is. Its
own `collect` into a `Vec` runs where std's `FromIterator` of a `Box<[T]>`, `Rc<[T]>`,
`Arc<[T]>`, `BTreeMap`, `BTreeSet`, or `BinaryHeap` calls it first, its own `eq` and
`partial_cmp` where `ne` and the tests on `partial_cmp` call them, and its own `by_ref`,
which std calls only from `Vec::splice`, which refuses one that overrides it, `position`, `max`, and the
others std never calls in place of another only where the crate calls them, and `fuse` reads on past the end of one that is a `FusedIterator`, as std's does. A caller calls `next()` for one item, or iterates
an `rs.Iter` with `for ... of`. Each `rs.Iter`'s `sizeHint` is std's `size_hint`, its upper
bound `undefined` for `None`, and a `collect` or `extend` into a `Vec`, `VecDeque`,
`BinaryHeap`, B-tree, or hash collection reserves by it where std's does, panicking with
std's capacity overflow past the most it holds. A tuple of these, of `String`s, `()`, or such
tuples, collects, unzips, and extends as std's tuple impls do: a 1-tuple's collection by
its own `extend`, and a longer one's each reserving the lower bound, in field order, before
each item's fields go into them in order, as `rs.tupleExtend(t, it, [rs.vecPart(2),
rs.STRING_PART])`. The crate's own `Default + Extend` collection keeps the defaults stable
Rust can't override: it reserves nothing, and takes each item by its `extend` of `Some(item)`,
through `rs.ownPart`. Any other collection in one is refused.
The crate's own collection is collected by its own `FromIterator`, which `collect` and a
`collect` into a `Result` or an `Option` of it call with the iterator, as std's do, and std's
adaptors that take an `IntoIterator`, such as `flat_map`, `flatten`, `chain`, `zip`, and
`extend`, iterate it by its own `into_iter`.

`clone` of one of std's iterators or adaptors is its `rs.Iter`'s `clone`, which reads on
from where the original was, independently of it, as std's `Clone` does; one over a hash
collection steps a fresh iterator of it past what the original read, as JS can't copy
where a `Map`'s iterator is. What it holds that the runtime can't clone, an item one that
owns its items holds, an adaptor's closure, or the crate's own iterator, it clones by
what the translation gives it, as `it.clone({ v: Item.clone$Clone })`. A closure clones as Rust
clones its captures: one that writes none of them, each of which only copies or clones
as itself, is its own clone, and another whose clone translation reaches is made by a
function of its captures, which makes the clone again of theirs, in order. `cycle` takes
the same clones, as `rs.iterCycle(it, { v: Item.clone$Clone })`, by which it clones its
iterator as it is made and again each time a clone ends.

A `str` is a JS string, which is UTF-16: `chars` steps by code point, and a search by a
`char`, a `&str`, a predicate (a closure, a function or a function pointer, or a `dyn FnMut`) behind any references and boxes,
or an array or slice of chars, as in `rs.strSplit(s, "/")`, takes the text or the
closure, which it calls on each char it reads, in the order std reads them, and drops a closure it owns as it
returns, or with the iterator it returns, as `split` does, leaving one it borrows to its owner. An offset it
returns, from `rfind` or `match_indices`, counts UTF-8 bytes, as Rust's does, and so does
one `&s[a..b]` or `&s[a..=b]` takes, which reads the string up to it and panics as std
does inside a char, where `get` gives `None`. A JS string can't change in place, so `String`'s edits, `push_str`, `truncate`,
`insert`, `retain`, `replace_range`, and the rest, assign the edited one to the local or
field the `&mut` borrows, taking and checking byte offsets as std does, and `retain` then
drops its closure. `+`, of a `String`, is JS's `+` of the moved text, as `+=` is, and of a
`Cow<str>` takes either side whole when the other is empty, as std's does. A `&mut str` is
the `rs.Place` of its `String` wherever that is, and a part of one, from `&mut s[a..b]`,
`get_mut`, or `split_at_mut`, is `rs.strPart(p, a, b)`, a place of its own that writes the
whole string anew, checking its offsets as `&s[a..b]` does. `as_bytes_mut`,
`get_unchecked_mut`, and `str::from_utf8_mut` are refused. `extend` and
`collect` read their iterator by `for_each`, as std's do, and of chars first reserve its
size hint's lower bound, which panics past `isize::MAX`. A `drain` holds the `&mut`'s
place and writes the string without the drained text as it drops, so one that
`mem::forget` leaks leaves it as it was, as std's does. A `String` keeps no capacity:
`with_capacity`, `reserve`, and `shrink_to_fit` do nothing, and `capacity` is refused. A `Cow` keeps its
variant in its class, so a `match` reads it, and `to_mut` of a borrowed one stores what it
owns in the `Cow`'s place before lending it: a `Cow<[T]>`'s clone of its slice, or a
`Cow<str>`'s text, which a `String` is, so the `&mut` borrows the `Cow`'s own place. A
`Cow<[T]>`'s `clone_from` of an owned one over an owned one clones into the `Vec` it owns,
keeping its buffer, as std's does. `Cow::is_borrowed` and `is_owned` are unstable, and
refused.

Functions and members are camelCase. Functions are monomorphized: each instance is its
own function, suffixed after a `$` by the items it is nested in below its module, its
trait, then its type's type and const arguments and its own, as in `roundtripLite$lite_group_Group`,
`Wrap.new$fmt_Pair`, `Pair.fmt$fmt_Debug`, `DecodeError.from$convert_From_coding_varint_BoundsExceeded`,
and `through$u16s` for a `through` declared in `fn u16s`. Each type, trait, and closure
in a suffix goes by its path: the crate's own by its module path, a dependency's by its
public path under its crate, as `bytes_Bytes`, and std's by its public path without its
crate or a module its item repeats, as `Option`, `fmt_Display`, and `io_Error`. A closure
goes by where it is defined, `hints_typed_closure1` for `hints::typed::{closure#1}`, with
the type arguments of what it is in, and an impl by its trait and self type. A reference
is `ref` or `mut`, a slice `slice`, and an array or tuple goes by its length, `array4` or
`tuple2`, each before what it holds, and a const argument by its value: `4`, `neg4`,
`true`, and a `char` as itself if an ASCII letter or digit, else as `u` and its hex code
point, `u1f600`. A suffix spells `&[u8]` as `ref_slice_u8`, but a reference to a
generic type in its module after the type's arguments: `chunk$bytes_Buf_u8_mut` for `&mut
Wrap<u8>`'s, apart from `Wrap<&mut u8>`'s `chunk$bytes_Buf_mut_u8`. The suffix depends on
the instance alone, and is there even when no other instance is, so reaching one renames
nothing; two instances it doesn't tell apart fail translation. A type or trait declared
in a function has its module suffixed alike, `copy.Pair$run`, and an async block's class
by where its function holds it, `blocksFuture$closure1`. An instance of a generic std
function is suffixed instead by a hash of its text, with what it calls by name, as
`map$1cf8u0y`: instances that print the same are one function, whichever of them
translation reaches. Types are translated once, generically over their type parameters
but not their const ones, which type nothing in TS: arrayvec's `ArrayString<4>` and
`ArrayString<16>` are one `ArrayString`, and each instance's functions read its own
`CAP`. So a function of a generic type is one instance's, declaring the instance it
takes as `self`:
`fmt$fmt_Debug_fmt_Pair(self: Wrap<Pair.Pair>, f: rs.Formatter)`. A reference or pointer
in a generic type to what only an instance decides the form of, such as `*mut T` or
`&'a mut T`, is an `rs.Ptr<T>`: the object, an `rs.Place<T>`, or an `rs.ByteSlice`,
which each read of an instance's field casts to what it holds. So is a run of a
parameter, whose `u8` instance holds bytes: `[T; N]` is an `rs.ArrayOf<T>`, a `T[]` or a
`Uint8Array`, and `[T]`, `Vec<T>`, and `Cow<[T]>` an `rs.SliceOf<T>`, an `rs.VecOf<T>`,
and an `rs.CowOf<T>`. An `Option<T>` or a `Poll<T>` of a parameter is an
`rs.OptionOf<T>` or an `rs.PollOf<T>`, a conditional type TypeScript resolves at each
instance to what that instance's own `Option` or `Poll` is, `T | undefined` or
`T | rs.None`, so a read needs no cast, and a value made into a field is cast to it, which
TypeScript infers the instance's arguments from. A type's module exports the
type by its own name, which a caller writes `Decoder.Decoder`, and a function whose name
and exports it as its own: `function new_` is `Decoder.new`. A module imports a function
or value by its name, and a type's module whole by the type's, `import * as Decoder`,
unless another module could declare one of that name, or the importer declares one:
then by its name after the
shortest end of its module's path no other such module shares, as `counts_Noisy` or
`option_unwrapOr$0r2bmpr`. The modules that could are counted over every item of the crate,
the crates it translates, and std's that translate, whether or not translation reaches
them, so reaching more renames no binding. A local is renamed `x_` only where its own
function refers to a binding `x`. A `Copy` struct, enum, tuple,
or array, or an `Option`, `Result`, or `Poll` holding one, is copied where Rust copies
it, since JS would share the object: by a function of `lib.ts` made for its type, such
as `copy$shapes_Point`, which copies what it holds by value and shares what it holds by
reference. Only a type the crate writes into in place, compares pointers to, or takes an `Arc`'s raw pointer to is copied,
or one holding such a type by value: a value of any other type never changes once made
and has no address that tells its copies apart, so it is shared. A write counts through a field, an element, a reference, or a box, and a mutable reference
a std function's parameters take, where the crate calls it or takes it as a value, or
one kept other than in a local, counts as a write into what it points at. A write over a type parameter counts for the type each instance translation reaches gives it, a closure's and one reached through a trait object included. A caller must
not change in place what it passes to or gets from the translation either.

A `&mut` to a value JS copies, which no JS value can point at, such as a number, an
`Option`, an enum, a tuple, a `Box`, or a `String`, is an `rs.Place` whose `v` reads and
writes where it points. Where it is only read and written through, in the function that
borrows it, it is the place it borrows, so a body that passes none on makes none. Passed
on, returned, kept in a field, or captured by a value, it is
`rs.place(() => x, (value) => x = value)` over that place, the objects and index it goes
through held as they were when it was borrowed; a reborrow of one is the same
`rs.Place`. A caller passes one as `{ v }`, or as any object whose `v` it reads and writes.
std's mappings take a `&mut` parameter as what it points at. So is a `&mut` to an `Arc` or a
`Waker`, pinned or boxed too, whose clones may be one JS object.

A `&mut` to any other object is the object. A whole write through it, `*r = v` or std's
`mem::replace`, `mem::take`, or `mem::swap`, assigns the place it borrows where that is
known, as a local, a field, an element, or a `MutexGuard`'s value, and otherwise writes
into the object in place: the fields of a `Vec` of a struct, a collection, or a
`Weak`, with what it held moved out into an object of its own. A guard, a context, a
pinned reference, or a binding's value, whose object is shared or opaque, is refused there.
A write of a value of no bytes, through a `&mut` or a pointer, reads and writes nothing.

A collection keeps Rust's key semantics. A hash map or set keeps a key that is a JS
number, string, or boolean in a JS `Map`, which compares those as Rust's `Eq` does, and
finds any other key, a 64-bit one past 2^53 included, through its translated `Hash` and
`Eq`; a B-tree orders every key by its translated `Ord`, calling `cmp` on the keys std's
does, in the same order, so a tree a `collect` sorted by a `lt` that disagrees answers as
std's does. std's impls are the runtime's: an
`Option` orders `None` first, an `Option` and a `Poll` hash their variant and then what they hold, an `Arc` compares, orders, and hashes as what it holds,
so a map of `Arc<str>` finds a `&str`, a `Cell` and a `RefCell` compare and order as
what they hold, a `RefCell` borrowing each, a one-time cell compares its `Option`, and
arrays, slices, and vectors, of bytes or of other elements, order by their elements and then their length, and compare and hash alike
whatever their form, so a map of `Vec<u8>` finds a `&[u8]` and one of `[u16; 2]` a `&[u16]`.
A `VecDeque` orders as they do, however its buffer wraps, and so does a `LinkedList`, which
also compares and hashes as its iterator does: each item by `==`, never `!=`.
Slices, arrays, `Vec`s, and `VecDeque`s compare against each
other as std's do, by their lengths and then each pair of items in order, through the
items' own `PartialEq<U>`, and its `ne` where the crate overrides it, which a tuple's and
an `Arc`'s `!=` call as well. What a composite holds compares, orders, and hashes through a
reference or a `Box` as std's impls forward it, by what it points at, or by the crate's
own impl on the reference or `Box` itself. A reference std makes to an array it holds, as a
collection's `keys`, `HashSet::get`, and `retain`'s closure, `Option::iter`,
`slice::first`, and `filter`'s closure over `[T; N]` items get one, is an `rs.Slice` of
its elements, an `rs.ByteSlice` for bytes, as a `&[T; N]` is elsewhere; a `&mut` writes
where std holds them. A `&[u8; N]` std was given and holds, as a `HashMap<&[u8; N], V>`'s
keys, stays the slice it was, and `extend` by references to arrays copies each into its
own array. A `&[T; N]` std is given as a `&T`, as `Option<&T>::copied`,
`slice::contains`, and `assert_eq!` take one, and a key a map looks up by a `&[T; N]` of
other values than bytes, is the slice's elements, the array itself where the slice is all
of it, and else a copy, or for bytes a view of them, which std reads, copies, and writes
through where the reference points; `mem::swap`, `replace`, and `take` through a
`&mut [T; N]` write each element in place. A `&mut` to a value a map holds that JS copies,
such as a number, from `get_mut` or the entry API, is an `rs.Place` whose `v` reads and
writes the map's slot. A hash map iterates in an order
of the runtime's, mostly insertion order, which std leaves unspecified, and hashes a key
with a hasher of its own, as std's keys its at random. So is how often the runtime
calls a key's `Hash`, `Eq`, or `Ord`. A map or set built with any `BuildHasher` but
`RandomState` holds it, which `hasher` lends, `Default` and `collect` make, and `clone`
clones before what the map holds; it drops before the map's entries, and at once from
`into_iter`, `into_keys`, and `into_values`, as hashbrown's does. No key goes through it:
the runtime's own hasher finds them, so it builds no hasher for the map. A lookup calls
`eq` of the key looked up by, given the key held, as std's does. Two maps or sets compare
as std's: their lengths, then each entry of the left looked up in the right, and its
value's `==` of the one found.

A `DefaultHasher` is std's SipHash-1-3, and `Hash` writes into it what std's impls write,
so `DefaultHasher::new`'s hash of a value is native's. `RandomState::new` keys it as std
does, from keys drawn once and the first raised by one for each state, drawn from
`crypto.getRandomValues` where std asks the OS. std's `Hash` impls call a crate's own
`Hasher` as std's do, method by method, length prefixes, `str`'s `0xff`, and an integer
slice's one `write` included, and std's provided methods it doesn't write itself write
through the others as std's do. So do they into a `&mut` or a box of a `dyn Hasher`, of
`DefaultHasher` or the crate's own, as std's impls forward each method to it. `BuildHasher::hash_one` of a crate's own builds its
hasher, hashes, finishes, then drops the hasher and the value. A hasher JS copies, such as
an enum, hashes as a struct one does, through the `rs.Place` its `&mut` is.
Its iterators are `rs.Iter`s, so std's adaptors run over them.

A value drops where Rust drops it, if dropping it does something JS can see: a `Drop`
impl, an `Arc`'s count, or a waker. Bodies lower from MIR, so each drop is where drop
elaboration put it. A panic drops nothing. A value without `[Symbol.dispose]` of its own, such as
an enum, a tuple, or a generic struct holding one of those over its type parameters, as
`hashlink`'s `LruCache` holds a `LinkedHashMap`, whose `Drop` impl has an instance per
type, drops through glue lowered from rustc's `drop_in_place` shim, a
function of its type's own (`drop$drops_Held`). `bool`'s, `Option`'s, `Result`'s, and `Poll`'s
methods and `cmp::max`, `min`, and their `_by` and `_by_key` forms lower from core's MIR,
so they drop what core's do, where core's do. The runtime
functions for std's collections, `Arc`, and the std functions that may discard an operand
(`OnceCell::get_or_init`, a map's `insert`) take what drops each value they may drop, and
drop it where std's do. So do iterators: one that owns its items, as a `Vec`'s `into_iter` or a
map's `drain` does, drops those it still holds by `[Symbol.dispose]`, an adaptor drops
what it skips or rejects, and a consumer that takes an iterator by value drops it:
through each `rs.Iter`'s own `fold` where std's folds, which drops each adaptor's closure
as its fold ends, before what it wraps, as std's does. The crate's own `fold` is given that
closure as a `Box<dyn FnMut>`, which drops it where its body does, and so are its own
`find`, `find_map`, `for_each`, `reduce`, and `all`, whose `find_map` is given what it finds
boxed. `try_fold`, `try_rfold`,
and `try_for_each` drop their closure as they return, leaving what the iterator holds to drop
with it. `scan` hands its closure its state, or the state's place where JS copies it, and
drops the state after the closure, as std's fields drop. A `Vec`'s `drain` leaves the vector
what precedes its range until it drops, then drops what it still holds and moves the tail
back, so one never dropped, as `mem::forget` leaves it, leaks the rest, as std's does. A
`splice` drains so too, then fills the gap from its iterator as std's does, reserving
for the tail by that iterator's lower bound and then by what is left of it, collected,
and drops the iterator once the tail is back; an `extract_if` takes out each element its
closure picks as it reads them, moves the rest back over the gaps once dropped, and then
drops the closure.

A trait object of the crate's own trait, a dependency's, or std's or a mapped crate's, such as
`dyn Hasher`, `dyn fmt::Write`, or `bytes`' `Buf`, behind `&`, `&mut`, `Box`, `Arc`, `Rc`, or a `Pin` of one, is an object with
a method for each function a call can dispatch to, its supertraits' included, named for
its trait as an impl's function is and taking what follows `self`: `shape.area$Shape()`.
The trait's module exports its type, `Shape.Shape` from `dyns.Shape.ts`, which a TS
caller can implement. A generic trait's type is generic over the trait's type parameters,
then the associated types its objects bind that no supertrait binds already, so
`dyn Source<Item = u8>` is a `Source.Source<rs.u8>`, and is typed as the trait declares
it, as a generic struct is: a call casts what it returns to the form its object's
arguments decide, such as the `rs.u8` of an `rs.Ptr<T>`. An `Option<Self::Item>` is an
`rs.OptionOf<Item>`, so an `Item` that can be `undefined` returns `rs.None`. One whose
method takes or returns another form at its arguments, such as the `rs.None` of an
`Option<Self::Item>` whose `Item` is a `MaybeUninit<u32>`, typed `number`, is refused. A coercion wraps the value in a class of its type's from `lib.ts`,
named for the type as a suffix spells it, `new of$dyns_Square(sq)`, which every trait's
object of the type shares, whose methods call that type's impls; one from a `&mut` to a
value JS copies holds its `rs.Place`, as `ofPlace$dyns_Square`, so `&mut self` writes
where the value is. An upcast, to `dyn Any` too, is the same object. Objects of two traits
of one name, with a method of one name, or of one generic trait at two arguments, are
refused for one type, whose class can't hold both, as are objects of an array, which a
box holds as the array and a reference as a slice of it. A `Box<dyn Trait>` drops what it holds through its `[Symbol.dispose]`, which
a TS caller's object may leave out. An `Arc<T>` unsized to an `Arc<dyn Trait>`, or to an
`Arc<dyn Future>`, is an `rs.ArcDyn`: the object a box of its value would be, and the
`Arc<T>` it was made from, whose one count both sides' clones and drops change, and the
last of which drops the value. `Arc::ptr_eq` compares the `Arc<T>`s. An `Rc` is an `Arc`,
and a `Pin` the pointer it holds. A `Weak<T>` unsized to a `Weak<dyn Trait>` keeps the
`Arc<T>` and makes the `rs.ArcDyn` as it upgrades, since the value of `new_cyclic`'s
`Weak` is not there yet as it unsizes. `Arc::from` or `Rc::from` of a box is an `rs.Arc`
of what the box is, which drops it with the last clone. A pointer to a struct whose last
field unsizes to a trait object, such as `RefCell<dyn Trait>`, is an `rs.tailDyn` view of
the struct it was made from, behind an `rs.ArcDyn` in an `Arc`, and two such pointers are
equal if their views are of one struct. A raw pointer unsized to a trait object, or to a
struct of one, is what a box of the object it points at unsizes to, which drops it as
`Box::from_raw` of it does, or what a `&mut` to its place does; one to a library value,
such as a cell, unsizes as each read finds the value there, and stays null or dangling.
One whose object would hold such a value as it is made, such as a `*mut dyn Iterator` of
a `*mut Range`, is refused. Pointers to trait objects don't compare, as each coercion
makes an object of its own. An object of std's or a mapped crate's trait holds only
the methods some call through one dispatches to, so each buffer adds those of its impls,
not all of `Buf`'s; a TS caller sees the same in its type. Hashing into a `dyn Hasher` holds
each of `Hasher`'s. A class of a std type's value calls the mapping of its impl, such as
`DefaultHasher`'s `Hasher`, or a `String`'s or a `Formatter`'s `fmt::Write`, and one no
mapping makes, such as a `Vec<u8>`'s `io::Write`, is refused where a call dispatches to it.
A `dyn Buf` holds no `chunks_vectored`, whose `IoSlice` has no JS value, so a call of it
fails.

A `dyn Any` is such an object with no methods, whose class `is`, `downcast_ref`,
`downcast_mut`, and `Box`'s, `Arc`'s, and `Rc`'s `downcast` check for: `a instanceof of$dyns_Square ?
a.v : undefined`. Its `Debug` writes `Any { .. }`, as std's does. A `&mut` one of a value JS copies is the class holding its `rs.Place`,
or the class itself behind a `Box`, as a `&mut dyn Trait`'s is, and `Arc::downcast` gives
back the `Arc<T>` the object was made from, or, of one made from a box, an `rs.ArcDyn` of
the value sharing its count.

A `dyn Fn`, `dyn FnMut`, or `dyn FnOnce` is the closure or function it was made of, which
a call calls as it does the closure, and which a box drops as it does the closure. One
that `Fn` or `FnMut` calls, boxed as a `dyn FnOnce`, is `rs.once` of it, which drops it
after the call, as an `FnOnce` closure's body drops what it owns, and a box of a
reference to one is `rs.lent` of it, which never drops it.

A `dyn Error` is such an object of `source` and its supertraits' `Debug` and `Display`,
`of$errors_Leaf`, whose `source` is `None` for a type that keeps `Error`'s own. `From` an
error into a `Box<dyn Error>`, which `?` converts by, makes one. Its `is`,
`downcast_ref`, `downcast_mut`, and a box's `downcast` check for the class as `dyn Any`'s
do, which a value upcast from the crate's own trait is of too. Its deprecated
`description` and `cause` are refused, and so is `From` a string, which boxes std's own
private error type.

A `dyn Iterator`, `dyn DoubleEndedIterator`, `dyn ExactSizeIterator`, or `dyn FusedIterator`
behind `&`, `&mut`, `Box`, `Arc`, or `Rc` is an `rs.Iter` of its items, `rs.iterDyn` of the iterator it was made
of, std's or the crate's own: it reads that iterator by the methods std's vtable holds,
such as `next`, `nth`, and `size_hint`, and folds it by `next`, as std's defaults read a
trait object, so a `count` or `last` of a boxed `Chain` drops its items in the order the
trait object's does. A box of any iterator, a trait object or not, is `rs.iterBox` of it
wherever it is read, behind a `&mut` too: it forwards `next`, `nth`, `last`, and what else
std's `Iterator` for `Box<I>` forwards, and reads the rest by `next`, `advance_by` and the
folds included, so a boxed `skip` of a zip skips through the zip's `nth` as std's does. A
box drops the iterator it owns, and the last `Arc` the one they share. `Arc::get_mut` and `Rc::get_mut` of a trait object lend its object while no other
`Arc` or `Weak` shares it, but a `dyn Any`'s, whose value `Arc::downcast` gives back.

A trait object of a trait with a const parameter, of a byte array, or behind any other
pointer, such as a struct's last field, is refused, and so is `Any::type_id`.

An `async fn` returns a future without running anything: an instance of a class of its
own (`sumFuture`), the state machine MIR's coroutine transform makes. Its fields are the
arguments and the locals held across an `.await`, and its `poll` resumes at the state it
stopped in. A poll after the future returned panics, and dropping it cancels it,
dropping what its state holds. An async closure's future borrows what the closure
captured, and that of a call that consumes it owns it, as rustc's by-move body does: a
class of its own (`closuresFuture$closure2_closure0_once`), which a closure that `Fn` or
`FnMut` call and that owns what drops makes through `rs.callOnce`, as std's functions and
a `dyn FnOnce` call it.
`rs.spawn(future, signal?)` runs one as a promise, polling woken tasks in a batch from a
microtask and yielding to a macrotask after 128 polls. Aborting `signal` drops the future
at once, or as the poll it aborts in returns pending, and rejects the promise with the
signal's reason; a future ready in that poll resolves it. A waker's identity is its `data`
and `vtable`, as std's is, and its vtable an `rs.WakerVTable` of the four functions std's
`RawWakerVTable` holds, which `RawWakerVTable::new` and a `static` of one make, `==`
compares, and `Waker::new`, `Waker::from_raw`'s `rs.VTableWaker`, calls on its data for
each clone, wake, and drop, which a function casts back to what it points at, such as an `Arc`'s `into_raw` it counts and takes back. `Waker::from(arc)` and `RawWaker::from(arc)`, of an `Arc` of the
crate's `Wake` impl, hold a count of the `Arc` and wake through the impl: their data is
the `Arc`, and their vtable the impl's, so clones and wakers made apart from one `Arc`
share them. `Waker::noop()` is `rs.NOOP_WAKER`, and a caller's own waker gives its own
identity, which each of its clones shares, and a vtable of what it does.

A file's items follow the Rust source, whatever
order translation reaches them in, so adding a root moves nothing it does not reach.
What a macro makes sits where the macro is called. The instances of one function sit
together, sorted by their suffix (`fmt_Debug_fmt_Pair` in `fmt$fmt_Debug_fmt_Pair`) and then by their
full type arguments.

Imports name the `.ts` file, which Node, Bun, and bundlers run as is. A `tsc` build sets
`rewriteRelativeImportExtensions`, or `allowImportingTsExtensions` with `noEmit`.

## Well-known crates

rust2-ts maps a few crates itself, as it maps std, so a consumer binds and configures none
of them:

- `bytes::Bytes` is an `rs.ByteSlice`: an `rs.CountedBytes`, one of the handles its
  storage counts as `bytes` does, which drops by `rs.bytesDrop`, or a plain one where it is
  static. Its constructors, `len`, `is_empty`, `is_unique`, `try_into_mut`, `slice`,
  `Clone`, `Deref`, `AsRef`, `PartialEq` with bytes, `Default`, `Debug`, and `From`
  between it and `Vec<u8>`, `String`, `&'static [u8]`, and `&'static str` map to
  `rs/bytes.ts`, panicking as `bytes` does, and so do `split_to`, `split_off`,
  `truncate`, `clear`, and `slice_ref`, whose panic names no addresses. A `String`'s
  bytes are in storage of its capacity, which JS keeps none of, so where any crate makes a
  `Bytes` or a `Vec<u8>` of them, every crate is refused what reads a `Bytes`' capacity:
  `is_unique`, `try_into_mut`, `truncate`, `clear`, `From` into a `BytesMut` or a
  `Vec<u8>`, and `put` of a `Buf` but a `BytesMut` or a slice into a `BytesMut`. `slice_ref` of
  bytes from other storage throws, since Rust may share what JS copied, as a `&'static
  str`'s bytes that `as_bytes` encodes anew. `Bytes::from_owner`
  reads its owner's bytes once, as `bytes` does, and an owner that drops is kept in the
  `rs.Shared` they count, which drops it with the last `Bytes` sharing them; one of an owner
  is never unique, so taking it back copies.
- `bytes::BytesMut` is an `rs.BytesMut`: a vector of its own, or a window of storage that
  the handles `split_to`, `split_off`, and `split` make and the `Bytes` that `freeze`
  makes share without overlapping, as `bytes`' do. Its constructors, `len`, `is_empty`,
  `capacity`, `freeze`, `split_to`, `split_off`, `split`, `truncate`, `clear`, `resize`,
  `reserve`, `try_reclaim`, `extend_from_slice`, `unsplit`, `try_unsplit`, `Clone`,
  `Deref`, `DerefMut`, `AsRef`, `AsMut`, `PartialEq` with bytes, `Default`, `Debug`, and
  `From` between it and `Bytes`, `Vec<u8>`, `&[u8]`, and `&str` map, with `bytes`'
  capacities and panics: each handle drops, so `reserve` reclaims storage once no other
  handle holds it, as `bytes`' does. `spare_capacity_mut` is an `rs.ByteSlice` of the
  storage past its bytes, which `set_len` takes in, panicking past the capacity as
  `bytes`' debug assertion does.
- `Buf` and `BufMut` of a `Bytes`, a `BytesMut`, a `&[u8]`, and a `Vec<u8>` map: the
  required methods, `has_remaining`, `copy_to_slice`, `copy_to_bytes`, each `get_` and
  `put_` of an integer or a float, each `try_get_` of an integer of 2 to 16 bytes,
  `put_slice`, `put_bytes`, and `put`, whose panics are
  `bytes`'. `put` of another `Buf`, such as the crate's own, a `Take`, or a `&mut` to
  one, reads it through its methods as `bytes` does, which into a `BytesMut` of no
  capacity copies it out whole by its `copy_to_bytes` and takes the copy's storage if no
  other handle holds it, then drops it. The provided `copy_to_bytes` of the crate's own `Buf`, whose `BytesMut` has
  the capacity it asks for, reads it chunk by chunk through its methods. The provided
  `get_` and `try_get_` of an integer of 2 to 16 bytes of any other `Buf`, such as a
  `Take`, read its chunk where that holds the integer, and copy it out through
  `copy_to_slice` where not, as `bytes`' do. `chunk_mut` is an `rs.ByteSlice` of the
  storage past the bytes, once it reserved 64 where there was no room, as `bytes`' does.
  An `UninitSlice`, `#[repr(transparent)]` over a `[MaybeUninit<u8>]`, is that slice's
  `rs.ByteSlice`, so its methods, which translate from `bytes`' MIR, write the storage
  of the buffer that lent it.
  The impls forwarding through a `&mut` or a `Box` translate from `bytes`' MIR, and the
  `BufMut` methods they don't forward map as the buffer's own, but `put` and `put_bytes`,
  which grow it as their provided bodies do, 64 bytes at a time once it is full. `put` of
  the crate's own `Buf` through them into a `BytesMut` is refused, as `bytes` advances it
  by the room left in the capacity.
  The rest of `bytes`, such as the other provided methods of the crate's own `Buf` and
  its `Buf` of a `VecDeque<u8>`, whose `chunk` is the deque's front slice, translates
  from its MIR, refused where that is.
- `hashbrown`'s `HashMap` and `HashSet` are std's, which wrap them, and map as std's do,
  hashbrown's own methods included: `allocation_size` is a slot and a control byte for each
  entry the capacity holds, and its `Entry` is std's. Its `HashTable` keeps each item
  under the hash the caller gives, a `u64` past 2^53 included, chained by the caller's
  equality, which the runtime calls on each item under that hash, rather than on those
  hashbrown's tag bytes pick. It never calls a method's `hasher`, as it never moves an
  item. `find`, `find_mut`, `find_entry`, `entry`, `insert_unique`, `retain`, `iter`,
  `iter_mut`, `drain`, `clear`, `len`, `clone`, `Debug`, and what reads and sets capacity
  map, and so do its entries' `insert`, `or_insert`, `or_insert_with`, `and_modify`,
  `get`, `get_mut`, `into_mut`, `remove`, and `into_table`. What reads hashbrown's
  buckets, such as `find_bucket_index` and `iter_hash`, is refused, as are
  `get_many_mut`, `extract_if`, and `try_reserve`, whose error is hashbrown's own. hashbrown's `DefaultHashBuilder` and
  `DefaultHasher` are std's `RandomState` and `DefaultHasher`: foldhash's state is seeded
  at random as std's is, so which hash a key gets is never observable, through
  `hashlink`'s default too, a crate's own `Hasher` over hashbrown's. Nothing of hashbrown
  translates, so `hashlink`, which keeps its entries in a `HashTable`, translates over the
  runtime's.
- `rand::rng()` is an `rs.ThreadRng`, whose `random` of an integer or `bool` and
  `random_range` of an integer range map to `rs/rand.ts`. Each value comes from
  `crypto.getRandomValues`, uniform as `rand`'s are, though no seed reproduces them, as
  none reproduces `rand::rng()`'s. An empty range panics as `rand`'s does. A seeded
  generator, such as `StdRng`, is refused, since its values would have to be `rand`'s.
- `tracing`'s events, `trace!` through `error!` and `event!`, are left out where they are
  written. Translated code installs no subscriber, and without one an event evaluates none
  of its fields and does nothing, so leaving it out is exact. Installing one, as
  `tracing::subscriber::set_global_default` does, is refused as any unmapped call is.

## Bindings

A binding stands in for what the subset refuses, such as `unsafe` code or a boxed future.
Its path goes from the crate root, or else from a crate it depends on by the name the crate
gives it, as `buf::Blob` for `buf = { package = "blob" }`. A crate it reaches only through
another goes by its own name, where one crate of that name is built.
Its file, relative to `ts.toml`, is copied to the same path in the output, beside the
runtime. It imports `rs.ts` as the output's modules do, `../rs.ts` from `bind/blob.ts`,
and may import the translated modules and npm packages too. Callers import it whole, as a
type's module, `import * as Blob from "./bind/blob.ts"`.

A bound type is the file's export of its name, `Blob.Blob`, with the Rust type's type
parameters. A call of a bound function, or of a function that an impl for a bound type in
the type's own crate defines, calls the file's export. The export is named as the
translated function would be, without the type's or the function's type arguments, so one
export serves every instance and a generic one is a generic TS function. `Blob::slice` is
`Blob.slice`, `<Blob as PartialEq>::eq` is `Blob.eq$cmp_PartialEq`, and
`<Blob as From<Vec<u8>>>::from` is `Blob.from$convert_From_Vec_u8`. A trait's provided function
that the impl doesn't override, such as `ne`, and a blanket impl's, such as `ToOwned`'s,
stay the trait's, as for any other type. A generic export sees a type argument only in the
values it takes, so a function with a bounded type parameter that no argument holds, such
as `fn zero<T: Default>() -> T`, or with a const parameter, can't be bound. `tsc` checks a
generic bound type's value at each of its instantiations translation reaches.

A type with a representation of its own, std's or a well-known crate's, can't be bound.

Translation can't see into a bound type, so a field, variant, or constructor of one is
refused, as are iterating one and assigning a whole one through a `&mut`. Its value is never `undefined` or `rs.Pending`, which `None` and
`Poll::Pending` are, and is an object unless the Rust type is `Copy`. A `&mut` to one
passes that object, which the binding's functions change in place. Where the type is
`Copy`, such a `&mut` is refused, since JS shares the value where Rust copies it, and a
binding never changes a `Copy` value in place. Nor does a binding change in place what it
takes by value or by `&`, or share with a clone what a `&mut` changes, and an integer it
returns is in its Rust type's range.

The check of the output also checks each export against the
translated signature of every instance translation calls, and each bound type's value. One
returning `()` is checked against `=> undefined`, not `=> void`, which accepts any value,
so its export is typed `: undefined`. A missing or mismatched binding fails the
translation at the Rust item's span, and the output is removed. Only the binding files translation reaches are copied, so one imports
another only where translation reaches both. A binding inside the output is refused, since
a translation replaces it.

A bound type keys a collection, alone or in a tuple, `Option`, or `Arc`, by the exports of
its Rust impls: `hash$Hash(self, state: rs.Hasher): undefined` and `eq$cmp_PartialEq` for a
`HashMap` or `HashSet`, `cmp$cmp_Ord` for a `BTreeMap` or `BTreeSet`, and
`partialCmp$cmp_PartialOrd` too for a B-tree's `range` and `extract_if`. A derived impl's are the same exports.
Each must agree with the Rust impl: `hash` writes what it writes, through `rs.hash8` to
`rs.hash128`, `rs.hashI8` to `rs.hashI128`, `rs.hashUsize`, `rs.hashIsize`, `rs.hashStr`,
and `rs.hashBytes`, so values `eq` finds equal hash alike, and
`cmp` orders as it does. A collection finds even a bound value that is a JS primitive by
these, never by `===`. A missing one fails at its impl, as any missing export does.

A bound type with `drop = true` drops where Rust drops it, through its
`[Symbol.dispose]`, which `tsc` checks it has. Without it, dropping one does nothing JS
sees, `tsc` checks it has no `[Symbol.dispose]`, and a type argument that drops is refused.

## Design

Output is ranked correctness first, speed second, and bundle size third: the
consumer's final JS, after their bundler tree-shakes and minifies it, so type-only
constructs such as brand casts and annotations are free. Readable output is not a goal.
Speed is a browser's: `just ts bench` runs Chrome and Firefox, and Node and Bun only as
stand-ins for V8 and JavaScriptCore, as Safari runs it; where any engine disagrees with
Chrome, Chrome decides.
It stays `.ts`, so `tsc` keeps checking the internals, and minifying is the consumer's
bundler's job. Types must be solid at the API boundary: the `roots` items and
`printable` types use Rust's branded types (`rs.u64`), while internals cast as freely as
they need.

- 64- and 128-bit integers are numbers while they are safe integers, which most values
  are: lengths, IDs, sequence numbers, and timestamps. `just ts bench` measured it
  against `bigint`, against limbs for every value, and against unboxed halves, two
  numbers in locals, fields, and arguments. It ties unboxed halves on the codec and
  beats `bigint` there in Node, Bun, and Chrome: varint encode 13-65% faster, frame
  decode 8-32%, and message decode 22-31%, though Chrome's 8-byte varint decode is
  25-45% slower; joining a varint's halves in one call, `rs.u64FromHalves`, makes that
  decode 1.05-1.2x faster in Chrome. A loop of sums and comparisons past 2^32 measured
  2-6x faster. Limbs for every value allocate an object each, which costs what a
  `bigint` does in V8, so they lost decode; unboxed halves would lower every value to
  two. Past 2^53 an operation allocates limbs, computed on their 32-bit words: sums,
  differences, comparisons, casts, products of 64-bit values or of two numbers, and
  division by a divisor below 2^32, with the rest of 128-bit arithmetic through `bigint`
  inside the runtime. A literal past 2^53 is written out as limbs, which cost what a
  hoisted constant does. Converting timestamps between timescales as moq-net does,
  products past 2^53 divided back down, takes 0.8-1.0x the hand-written `bigint` route's
  time in Chrome in `u64`, and 1.2-1.5x in `u128`. `u128` and `i128` as
  native `bigint`s measured 1.4-1.8x slower than limbs on that in Chrome and 1.2-1.5x in
  Node, and within noise in Bun, for a bundle 80 bytes larger. The runtime adds 6 KB to
  the codec's bundle, 2 KB gzipped. Brands tell `u64` from `i64` and `u32` from `usize` at no cost, since
  TypeScript erases them; a required brand would cast every literal, so a plain number
  passes for any type.
- A struct is a plain object, and each function of a type, trait impls included, is its
  own function in the type's module, so a caller's bundler drops every one it never
  calls: no bundler drops a class's unused methods or an object's unused properties. A struct that
  may drop, or holds a type parameter that may, keeps a class for its `[Symbol.dispose]`.
  `just ts bench` measured it against classes everywhere and against a class facade on
  the root types over free functions: all three within noise in Node, Bun, and Chrome.
  A caller of moq-net's translation that decodes a group header bundles to 26.5 KB
  minified and 8.3 KB gzipped, against 32.3 and 9.5 KB as classes; one that encodes
  one, to 22.5 and 7.5 KB against 31.6 and 9.5 KB, each with the whole runtime, which
  did not shake then. The facade was the largest,
  since its methods keep every function they call. RxJS, Zod Mini, Valibot, Effect,
  and date-fns made the same move for the same reason.
- A type's functions are a module of their own, which callers import whole and call as
  `Decoder.slice(dec, 4)`, as Effect's `Option.map` reads: Bun, esbuild, and Rollup drop
  every function of an `import * as` the caller never calls, and the bundles measured
  the same as flat names (`decoderSlice`) within 30 bytes. A barrel that re-exports a
  type's module (`export * as Decoder`) still shakes in Bun and Rollup but not in
  esbuild ([evanw/esbuild#1420](https://github.com/evanw/esbuild/issues/1420)), which
  keeps the whole module: 10.2 KB gzipped against 8.3 for the decoding caller.
- The runtime is free functions too, flat exports of `rs.ts` named for what they act on
  (`rs.u64Add`, `rs.iterMap`, `rs.fmtPad`), so a bundler drops every one translated code
  never calls. `rs.ts` re-exports its parts with `export *`, since esbuild keeps a whole
  `export * as` namespace. A method stays only where a call dispatches on its receiver:
  an iterator's `next`, a future's `poll`, a waker's, and `[Symbol.dispose]`. Each
  integer type's functions pass their bounds or width to shared helpers, with a wide
  type's number fast path inline in its own: through the shared helper alone, V8 took
  up to 1.7x as long on frame encode. A caller of moq-net's translation that decodes a
  group header bundles to 6.9 KB minified and 2.2 KB gzipped, against 21.6 and 6.9 KB
  with the runtime's namespaces and methods; one that reads a varint, to 4.7 and 1.5 KB
  against 19.4 and 6.2. `just ts bench` measured it within noise of them in Node, Bun,
  and Chrome: 1.01, 0.97, and 1.00 times their time at the geometric mean, where the
  A/A route measured 1.00, 0.97, and 0.99. The whole runtime, every export kept, is 52
  KB minified against 36.
- A generic type is one interface, class, or union, the type TS callers name, and a
  function of it is one instance's, typed by its `self`. A type per instance
  (`Wrap$Pair`) would rename the type for callers, and TS generics with trait
  dictionaries would undo monomorphization.
- `None` is `undefined`, so a TS caller reads an `Option` with `?.` and `??`, except where
  the `Some` can already be `undefined`: there it is `rs.None`, as `Pending` is
  `rs.Pending`, chosen by the type alone, so every function agrees on it. A `rs.None`
  for every `Option` would cost callers `?.` and `??` everywhere for the few that need it.
- A `Cow<str>` is a JS string, as `str` and `String` are, rather than an object tagged by
  its variant: no `Cow<str>` in moq-net or kio tells its variants apart, and a tag would
  wrap every value and unwrap it at every read.
- A `Cow<[T]>` is told apart by class, an owned one bare and a borrowed one in an
  `rs.Borrowed`, so it clones and drops as std's does and only the borrowed side pays a
  wrapper. Untagged, as `Cow<str>` is, `to_mut` would clone on every call and a `match`
  would be refused; tagged, every `Cow` would be wrapped.
- A `Duration` keeps std's fields, whole seconds and nanoseconds, rather than one count
  of nanoseconds, so its overflow, `checked_*`, and ordering are std's exactly, and a
  lowering of its methods from core's MIR would read the same fields.
- `Result` is a plain `{ ok, value } | { ok, error }` object rather than an exception, so
  `?` and `match` keep Rust's control flow exactly. One whose payload is `undefined`, such
  as `Ok(())`, is one frozen object every evaluation shares, since nothing writes into a
  result in place and engines don't remove one returned through a call they don't inline:
  `just ts bench` measured the codec's encode workloads at 0.90 times their time without
  it in Chrome, 0.75 in Firefox, and 0.75 in Bun at the geometric mean, with A/A within
  0.03 in Chrome and Node within its noise. Sharing a result of any other constant, such
  as `Err(DecodeError::Short)`, measured no faster on the decodes that return it.
- A `&[u8]` is an `rs.ByteSlice` rather than a `Uint8Array` view, since a view costs
  several times what an object does to make in V8 and JavaScriptCore, and a varint's
  decode splits its slice once to four times. `just ts bench` measured the codec's decode
  workloads at 0.75 times the views' time in Chrome, 0.81-0.84 in Node, and 0.82-0.85 in
  Bun at the geometric mean, over two runs whose A/A route measured within 0.05 of them,
  and encode no slower. A `&[u8; N]`, such as `split_first_chunk`'s chunk, is an
  `rs.ByteSlice` too, its bytes read in place: the decode workloads then took 0.72 times
  a view's time in Chrome, 0.67 in Firefox, 0.87 in Node, and 0.71 in Bun at the
  geometric mean, with the A/A route within 0.03, and encode no slower.
- A run of bytes `as_chunks_mut` lends is a `Uint8Array` view, which a whole write of
  another run can't detach, so a read of one by value copies it (decided with the
  maintainer on 2026-11-03). moq writes its runs by element and reads none whole, so it
  pays nothing. `just ts bench`'s `packets/` workloads, moq-mux's continuity counters over
  188-byte packets, measured the copy at 430 ns a packet in Node, 84 in Bun, 1930 in
  Chrome, and 800 in Firefox, against 280, 114, 1300, and 375 for the hand-written
  route's `slice`.
- A local built of byte slices and scalars, in tuples, `Option`s, `Result`s, and
  `ControlFlow`s, is a JS value per part: a slice's `data`, `start`, and `length`, and an
  enum's variant as a flag, when nothing borrows or writes into it and only std's splits,
  `ok_or`, `?`, aggregates, and copies assign it. A read that takes it apart reads its
  parts; any other makes its object there, and a local whose object would be made more
  often than before stays one. `just ts bench` measured the codec's varint decode at 0.49
  times the objects' time in Chrome, 0.57 in Firefox, 0.64 in Node, and 0.56 in Bun at
  the geometric mean, and frame decode at 0.61, 0.56, 0.70, and 0.61, with the A/A route
  within 0.04 and encode no slower. A tuple or `Result` of numbers alone is held so only
  where it meets a slot below, and its object, such as an `rs.Ok`, counts as made at each
  read that makes it. An `Option` of a number is one part, that or `undefined`, as its JS
  value is. A return is still a `ByteSlice` but for the `Result`s below.
- A function only translated code calls takes each `&[u8]` or `&[u8; N]` parameter as
  three, its `data`, `start`, and `length`, which its body holds as that local's parts:
  not a root, which the outside calls, a closure, or an `async fn`. One whose object the
  body would make anyway, to borrow it, to pass it to a function taking it whole, or at
  all, it takes whole: as parts, the callee would just make the object instead of the caller.
  That made Chrome's `btree/` workloads 1.06x `main`'s time through a helper that
  rebuilt the slice it was passed. What a body makes depends on what its callees take, so
  the functions a call reaches settle together, recursion included, each dropping the
  slices it would make until none does. The function as a value is an arrow that takes
  the slice whole. `just ts bench` measured parts
  no faster than passing the slice whole, since V8 already removes what an inlined call
  doesn't keep: Chrome's varint decode at 0.98-0.99 times the time and frame decode at
  1.01-1.05 over two runs, with the A/A route within 0.05, Firefox and Bun within their
  noise, and Node's varint decode at 1.11-1.15.
- A function of the crate's that only translated code calls and that returns a `Result`
  of byte slices and scalars, or of numbers alone, such as `Result<(u64, &[u8]), E>`,
  `Result<u64, E>`, or `Result<Option<u64>, E>`, whose error is one JS
  value that is never `undefined`, writes the `Ok`'s parts into a module-level slot and returns the error or
  `undefined`. The slot is shared by every function returning that payload type, in
  `lib.ts` and named for the type as the shims are, as `out$ref_slice_u8` or `out$u64`, so
  that reaching another function renames nothing.
  The function writes it at its JS `return`, after its drops, and its caller reads it at
  once, so one that returns another's result as is returns the call. The slot's fields
  start as `undefined`, so V8 keeps them tagged. When the `Ok`'s first part is a number,
  the function returns that, or `undefined` for an error, and writes the error into the
  slot instead: V8 boxes a number past 2^31 that a function too big to inline writes into
  the long-lived slot, which made Chrome's 8-byte leading-ones varints 1.06-1.09x `main`'s
  time. A slice's bytes stay in the slot, since returning them gave back frame decode. The
  function as a value is an arrow that rebuilds the `Result`.
- A `BTreeMap` is std's B-tree, ported: nodes of up to 11 keys with their values,
  children below internal ones, each searched from its first key by `key.cmp(k)`, split,
  merged, and evened out with a sibling where std's are, built by `collect` and
  `split_off` as std's builds them, and cloned node for node, so a key's own `cmp` runs
  on the same keys in the same order as std's. A search by another type the keys lend by
  the crate's own `Borrow` is an `rs.By`, which orders that type by its own `cmp` against
  each key's `borrow`, as std's does. Iterators climb and descend leaf edges as
  std's do. On `just ts bench`'s `btree/` workloads it takes 0.8-1.0x the time the B+
  tree of 64-key nodes it replaced did in Chrome, 1.0-1.3x in Node and Firefox, within
  their noise, and 1.3-1.6x in Bun, from 16 keys to 4096; bisecting a node's keys measured
  no faster. A hash map's JS primitive keys
  are a JS `Map`'s, the engine's own hash table; its other keys go into a second `Map`
  by their translated hash, chained by `Eq`. A hash map, set, or table holds only what std
  guarantees of its capacity, at least its length and what `with_capacity` and `reserve`
  ask for, doubling as inserts grow it, and keeps it as it holds fewer or clears, as a
  table keeps its allocation; hashbrown's exact bucket counts are not followed (decided
  with the maintainer on 2026-11-01). It holds one, as `cap`, only where an operation
  sets it or a removal reads it, so an insert never reads or writes one, and reading it
  doubles it from 3 while short of the length. `VecDeque` is a ring buffer over an array
  whose length is std's capacity, a `Uint8Array` of bytes, and each step places its items
  where std's does, so `as_slices` splits them as std's; one made from a `Vec` takes the
  capacity held for the `Vec`, and a `Vec` made from one takes its. A `Vec` and `BinaryHeap` hold std's
  capacity exactly: an `rs.Vec` holds the capacity, as `cap`, that the last step that
  fixes it set, as `with_capacity`, `reserve`, `shrink_to`, a `clone`, or a `collect`
  does, and, as `peak`, the most it held before a step that shortened it, so a push never
  reads or writes either. Reading it doubles `cap` while short of the longer of its length
  and `peak`, from the least std's `RawVec` allocates for the element size, as a run of
  pushes grows std's. Holding the capacity on each push instead took up to 1.7x as long on
  `push/vec`, and tracking it on each pop up to 2.8x on `push/stack` (decided with the
  maintainer on 2026-11-03). A `Vec<u8>`'s is its buffer's length, so its
  `spare_capacity_mut` is an `rs.ByteSlice` of the buffer past its length, which `set_len`
  takes in, panicking past the capacity as std's check does, which can't unwind; any other `Vec`'s is refused,
  as no JS array holds room past its length. Their `try_reserve` and `try_reserve_exact`, and a
  `String`'s, give `Ok` where `reserve` makes room and `TryReserveError`'s
  `CapacityOverflow` where it panics, and `AllocError`, of the layout std's asks for, only
  where JS fails to allocate a byte `Vec`'s or `VecDeque`'s `Uint8Array` (decided with the
  maintainer on 2026-11-03). A `LinkedList` is nodes linked both ways, as std's are,
  so `append` is O(1) and `split_off` walks from the nearer end; each node is the place
  `iter_mut` and `front_mut` lend of an item JS copies. Dropping a whole hash map or set drops its entries in the
  runtime's order, not hashbrown's: std leaves that order undocumented
  ([#272](https://github.com/moq-dev/rust2/pull/272)). Maintainer's decision on 2026-10-30.
  A whole B-tree drops as std's `drop_tree` does, node by node in postorder, and its
  `IntoIter` in key order, as std's does.
- A slice's `sort` and `sort_unstable` are std's driftsort and ipnsort, ported to the
  runtime with the choices std makes by the element's size on wasm32 and whether it is
  `Freeze` and `Copy`, so each calls the comparator in std's order and leaves equal
  elements where std's does: JS's `sort` moves `undefined`, a `None`, to the end without
  comparing it, and each engine orders its comparisons differently. A byte slice's sorts
  by its own `Ord` are its typed array's, whose order nothing observes. `sort_by_cached_key`
  sorts its keys paired with their indexes by that `sort_unstable`.
- A `BinaryHeap` is std's heap in an `rs.Vec`, with std's sifts ported to the runtime: std's
  move elements through raw pointers into its `Vec`, which no body of std's lowers
  through, and the sifts decide which `PartialOrd` tests run and which of equal elements
  leaves first. `Vec::drain`, `splice`, and `extract_if`, and `VecDeque`'s rotations
  and `make_contiguous`, are ported for the same reason: std's move elements through raw
  pointers. The sorts, `max` and `min`, the heap,
  and `is_sorted` take an integer's tests as the runtime's `rs.lt`, `rs.le`, `rs.gt`,
  and `rs.ge` rather than through its `partial_cmp`, `rs.cmpLt(rs.cmp)`: on `just ts
  bench`'s `order/` workloads that took Firefox 0.8-0.86x the time on sorts and
  `max`/`min` and Chrome 0.66x on the heap, and Chrome the same on the rest.
- Iterators are the runtime's lazy classes rather than generators or ES iterator
  helpers: std's adaptors would lower from std's own bodies, whose slice iterators are
  raw pointers, ES helpers lack most of them
  (`rev`, `zip`, `peekable`), and a `next` returning an item or `rs.DONE` allocates
  nothing per item. `just ts bench` measures chains of `map`, `filter`, `any`, `skip`,
  and `take` 1.4-8x faster than the same ES helpers in Node, Bun, and Chrome.
- Bodies lower from MIR, not THIR: drops are where drop elaboration put them, exact by
  construction, with no `finally` per scope or `undefined` left by a move. Async bodies are
  the coroutine transform's state machines, a class each. Against the THIR lowering it
  replaced, MIR closes about 1,100 of the 1,400 drop refusals in moq-net's census, and
  runs `just ts bench`'s codec 2-6% faster at the geometric mean and the async handoff
  1.4-5.9x faster at depths 1 to 16 in Node, Bun, and Chrome, in bundles no larger
  gzipped. Blocks become nested blocks, loops, and `if`s by
  Ramsey's "Beyond Relooper", except a coroutine's resume function, which enters loops
  in the middle and becomes a loop over a `switch` on the next block. Temporaries
  assigned once and read once fold back into the expression that reads them unless that
  would reorder an effect.
- `Option`'s and `Result`'s combinators lower from core's own MIR rather than runtime
  functions, each of which restated core's drops by hand. Each closure gets
  its own instance, keeping call sites monomorphic, and instances that print the same are
  merged into one. An `Ok` rebuilt from a stable `Result`'s payload is that `Result`,
  without which Node ran the codec 1.31x slower. Against the runtime functions, the codec
  and async handoff bundles are within 25 bytes gzipped, and `just ts bench` measures
  them within the A/A route's spread in Node, Bun, Chrome, and Firefox.

## Not yet

Byte reinterpretation, pointer addresses, pointer arithmetic on a value JS keeps as an object (other than wrapping arithmetic on a null or dangling pointer), memory from `alloc` other than an array of values JS copies, and unions whose
fields overlap in different types ([unsafe](/quest/m3/ts-unsafe/README.md)), and trait objects other than
`dyn Debug`, `dyn Display`, `dyn Future`, `dyn Any`, `dyn Error`, `dyn Fn*`, `dyn Buf`, `dyn Iterator` and its subtraits, and the translated crates' traits'
behind `&`, `&mut`, `Box`, `Arc`, `Rc`, `Weak`, or `Pin` ([trait objects](/quest/m3/ts-dyn.md)), fail translation. Until they translate, an item that needs one, such as a `Box<dyn Error>` made `From` a string, binds to
hand-written TS ([bindings](#bindings)).

These fail translation: `f16` and `f128`, a `Poll` of a `Poll` of a `Poll`, a future whose output can be pending, a closure that owns a value
that drops given to a std function that keeps it other than in an iterator or a
`poll_fn`, or whose mapping doesn't drop it where std's does, a `Cow` of other than `str` or a slice, a std
function without a mapping of its own that makes or reads an `Option` whose `None` is
`rs.None`, an `Option` of an `Option` of `()` or another
`Option`, an instance of a generic type whose field is an `Option` of a type parameter
that can be `undefined` though TypeScript's type of it can't, as a `MaybeUninit<u32>`
can, or an `Option` or a `Poll` of another form of it whose instance can be `undefined` or pending, a `String`'s `capacity`, looking up a key by
another type it borrows as through the crate's own `Borrow`, a `&mut` to a value JS
copies that std makes, other than a map's, or hands a closure, as `iter_mut` and
`and_modify` do, one into a `Result`'s payload passed on or kept, the adaptors
without a mapping in `src/lower/iter.rs` (such as nightly's `intersperse`), an
own `impl Iterator` or `DoubleEndedIterator` that overrides a provided method std calls in place
of another other than those, such as `map`, which `flat_map` calls, or whose own
method taking a closure tells it, or its accumulator or what it finds, apart from another, by an
`Option` or a `Poll` of the accumulator, a call or constant generic over them other than calling
or dropping the closure or passing them to std's iterators outside the crate's own types, or a
drop of the accumulator or what it finds, or whose own `find`, `rfind`, or `is_sorted_by` gives
its closure a reference to a `[u8; N]`,
a pointer's address (`{:p}`) and `Debug` of a `Waker`, a `Context`, a `RawWaker`, or a `RawWakerVTable`, which writes addresses, a `str` pattern other than a
`char`, a `&str`, a predicate behind any references and boxes, or chars, a mutable `str` slice, a copy of a closure that changes
what it captured, `mem::swap`, `replace`, or `take` of a reference's whole pointee,
assigning an `Arc`, a waker, a `MutexGuard`, a pinned reference, or a future through a
reference, a switch on or a field of another crate's type without a mapping, and std
functions without a mapping in `src/lower/library.rs`, but for `bool`'s, `Option`'s,
`Result`'s, and `Poll`'s methods that take no `&mut`, and `cmp`'s `max` and `min` forms,
which lower from core's MIR.

## Tests

```bash
just ts test                  # translate each fixture, compare, type-check, and diff
UPDATE_EXPECT=1 just ts test  # rewrite the snapshots in each fixture's expected/
just ts moq                   # translate moq-net's codec at the model fixture's pinned revision
just ts census                # census moq-net and kio at the same revision
```

Each fixture under `fixtures/` is translated and compared with its `expected/` snapshot,
then type-checked with `tsc`. A fixture with a `harness::run(&[u8]) -> Vec<u8>` is also
built natively for wasm32 and run beside its translation on the same inputs by
`tests/differential.ts`, which fails on any difference in output or panic messages. A
harness that names its cases draws each case's inputs from a stream seeded by its name,
so adding, removing, or reordering a case changes no other case's inputs.

- `codec` vendors moq-net's sync codec, which a harness outside the crate could not reach,
  and writes its errors' `Display` and `Debug`. It is also translated without
  `bench::run` and then with it as the first root, which must keep every line in order.
  Bun bundles a caller of `Decoder.u8` and `Decoder.new` through `import * as Decoder`,
  which must keep only those and what they call.
- `model` depends on moq-net at the moq pin, its `rev`, which `just ts moq` and
  `just ts census` check out too, and translates with it as one program. It is a
  workspace of its own, so its lockfile keeps moq's checkout out of the package's inputs. Its harness
  runs what of the model translates with nothing refused or bound, `Timestamp` and
  `Timescale`: their constructors, conversions, `u128` reads, sums, differences,
  temporal order across scales, `Duration`s, and `Debug`.
- `lang` is the language, a fixture per module under `fixtures/lang/`, each module's doc
  saying what it covers; `fmt`, `generics`, and `shapes`, which share types, share
  `shapes`. Each crate's `cases` names what it runs, which the first two bytes of an input
  pick through the `harness.rs` they share. Its differential test runs through `lang.ts`,
  a hand-written caller, which a fixture's own `caller.ts` extends with the inputs its
  cases name: `ints`' adds every 64- and 128-bit case over each pair of values next to a
  power of two, where the runtime switches between numbers and limbs, and its limbs carry
  and borrow, with products of those values at 128 bits, 128-bit quotients by divisors
  below 2^32, and their halves joined back, and `shapes`' prints a `printable` error
  nothing translated prints. `shapes/brands.ts`, another caller, mixes integer types where
  `tsc` must refuse them. `just ts test lang::ints` runs one.
- `names` is translated from `run`, then with `added` as its first root, which reaches
  another instance of what `run` reaches one of, each sorting before it: a generic method's,
  a closure's, an impl's of another trait, a std function's that merges with `run`'s, and
  an impl's over a deeper `&mut`, beside a trait object's module named as a type's that
  `run` imports. Every line of the first must stay, in order.
- `share` holds a `Copy` aggregate whose only write in the crate makes it copied, where
  `lang`'s other writes into the same type would hide a write the analysis misses: a
  byte array overwritten whole through a type parameter, or, under its features, by std
  called directly or through a pointer a generic function calls.
- `task` trims moq's kio to a dependency of its own, `kio`, reached across crates as moq-net
  reaches kio: a channel of `Arc`ed state, waiters, and wakers, kio's `Task`, `Pending`, and
  `Waiter::poll_future`, and a `Consumer::poll` whose closure is itself pending, a
  `Poll<Poll<_>>`. Async functions, async closures called directly and through each
  function trait, and std's future helpers run over it, with wakers of the crate's `Wake`
  impls, beside values that drop inside std's types. Its caller is hand-written TS (`tests/task.ts`): a FIFO executor that runs a
  script of spawns, sends, polls, and drops, and logs every poll, wake, and drop, as the
  fixture's native executor does. The logs must match byte for byte.
- `stock` calls `bytes`, `hashbrown`, `rand`, and `tracing` with no config, `hashlink`
  over `hashbrown`, and `rangemap` over std's B-tree, against native Rust, whose
  `rand::rng()` reads the fixture's own getrandom backend on wasm32. It checks `Bytes`'s
  slices and panics, how each is made and printed, that no event evaluates a field, and
  that each random value is in its range, every value of a small range drawn. It splits,
  freezes, grows, and writes a `BytesMut` by ops its input picks, ending with every
  piece's bytes and capacity, after the other handles of its storage dropped or not, and
  reads and writes through `Buf` and `BufMut` generically, through a `&mut` and a `Box`
  too, a buffer of its own and a `VecDeque<u8>` wrapped at each slot included, ending with
  each vector's capacity. It copies bytes out of `Chain`s of each kind of buffer, nested and as
  `dyn Buf`s, at either end of each half and past the end. It changes hashbrown's maps and sets, keyed by JS primitives and by
  tuples, and `HashTable`s of tuples and of numbers, whose places it writes, under hashes
  that collide and that pass 2^53, checks their capacity by std's guarantees, and writes a
  `hashlink::LinkedHashMap` in its own order. It inserts, removes, looks up, overlaps, and
  gaps `rangemap`'s maps and sets, which search their B-trees by what each key lends by
  rangemap's own `Borrow`. `caller.ts` names the inputs that read a `BytesMut` past
  where `bytes` shares a vector's storage, 128 MiB, drop each pick of a storage's
  handles before reserving, and read a deque across where its ring wraps and past its end.
- `bind` binds `buf::Blob`, a renamed dependency's type with `unsafe` inside, and a function and a type of its own that read through raw
  pointers, the type dropping an `Arc` where Rust drops it, to the TS in `bind/`. It keys
  hash and B-tree collections by `Blob` and by `key::Nibble`, a JS number equal to another
  by its low bits alone, through their bindings' `Hash`, `Eq`, and `Ord`. Its
  differential test runs them against native Rust. `wrong.toml` binds TS that doesn't match
  the Rust signatures, which `tsc` refuses at each item, with the errors in `wrong.txt`.
  `refused.toml` holds bindings the config can't make, with the errors in `refused.txt`.
- `crates` translates with `crates/dep`, which writes into a `Copy` type only the package
  copies, ranks the package's type in a generic function, drops, iterates, and reads
  through a raw pointer behind a safe twin under `rust2_ts` and in a function
  `bind/last.ts` stands in for. `bind/slots.ts` stands in for a type whose method alone
  writes into an array the package copies. Its differential test runs them against native Rust.
  `refused.toml` drops the binding, its `rev` feature makes the dependency's iterator run
  from either end, and its `shared` feature adds a module of the dependency's name, with
  the errors in `refused.txt`, `rev.txt`, and `shared.txt`.
- `reject` holds constructs outside the subset, a module per kind of construct. Its
  expected errors are `expected.txt` and its census's spans `census.txt`, each line keyed
  by the item it points into and sorted, so cases added to different items don't conflict.
  Its roots in `ts.toml` stay sorted through taplo.
- `census` holds a few constructs translation refuses for good, whose census, with its
  totals, is `census.md`.

`tests/spawn.ts` checks that `rs.spawn` yields to a timer after its 128-poll budget,
rejects when a future panics as it drops, and drops a future when its signal aborts:
before its first poll, while pending, and from within its own poll. `tests/retain.ts` runs a collection's `retain`
and `Vec::dedup_by` over every keep mask of up to 6 items, and `retain` checks that each
leaves and drops what std's does.

## Bench

```bash
nix develop .#bench --command just ts bench  # report to target/bench/report.md
```

The codec fixture's `bench::run` holds the cases moq's own TypeScript benchmarks time
(`js/net/bench/varint.ts` and `lite-varint.ts`), at the pinned moq revision, the same
frame headers decoded as they arrive a few bytes at a time, which prices a decode that
comes up `Short` and retries, iterator chains over bytes, a `BTreeMap` of 16, 256, and
4096 keys, and timestamps converted between timescales as moq-net's `Timestamp::convert`
does, whose products pass 2^53, then added by its `checked_add` and ordered by its `Ord`,
whose cross products do too, integers sorted, searched for their greatest and least,
and kept in a `BinaryHeap`, TS packets numbered through `as_chunks_mut` as moq-mux
numbers them, each also read by value, a copy of its bytes, and frames' `Bytes` split off a
group's, cloned, sliced, and dropped, as moq-net's model hands them to its consumers. Each runs four ways: natively, as wasm32 at the
size-optimized profile moq's WASM shipped with, hand-written (moq's `js/net` in `bench/moq.ts` and its `Timestamp`
in `bench/timestamp.ts`, ES iterator helpers in `bench/iter.ts`, a sorted array in
`bench/sorted.ts`, `bigint` in `bench/rescale.ts`, as moq's `js/hang` converts
timescales, typed arrays and a heap of its own in `bench/order.ts`, offsets into one
`Uint8Array` in `bench/packets.ts`, and `subarray`s in `bench/frames.ts`), and translated; every route's output must match
native Rust's first. The JS routes run minified, the same bundles whose size it reports,
in Node, Bun, Chrome, and Firefox, with the WASM route's copies across the boundary timed
apart from its compute, and an A/A copy of the
translation to show the noise. Each route runs alone in a worker of its own, since routes
in one realm share type profiles and run slower loaded second, and each round starts every
route's worker afresh: a worker keeps the inlining and deopts it settles into for the whole
run, which put identical copies of a route up to 1.6x apart in Chrome, so a round is one
sample of each workload from fresh workers. Each round warms up and times the routes and
workloads in a new random order, so no route keeps a slot that runs it faster or slower
than identical code in another; the report records the seed. It also records the host and engine versions,
and each route's bundle size raw and gzipped. It sets no threshold; the nightly workflow
runs it and posts the report to its summary. Each browser's page reports each step it
starts, and one that runs 30 s fails the run, naming it.

The task fixture's `bench::run` times an async handoff: a value sent to a task parked on
its channel, read through 1, 4, or 16 async functions. Its translation runs polled at
once, as natively, and on `rs.spawn`; its hand-written route is the same handoff with
promises, as moq's TypeScript writes it.

An A/B run times another translator's output as one more route: run `just ts bench` in
the other checkout, then pass its output directory, absolute or relative to `ts/`.

```bash
nix develop .#bench --command just ts bench --route main="$HOME/rust2-main/ts/target/bench"
```

The route runs both fixtures' translations from that directory's `translated/` and
`task/`, so a hand-edited translation can be timed by copying the directory and editing it.
Read its gap from `translated` against A/A's, the noise of the same run. No route holds a
fixed slot, so a gap past that noise is the change, and no reversed run is needed. Some
workloads still settle into one of two speeds per worker, up to 2x apart, so on a 24-core desktop
their A/A stayed 6-35% wide while the median workload's was 1-3%. The report's
A/A line also gives the noise of a family's geometric mean, the steadier gauge.

The bench shell adds Chromium and Firefox.
