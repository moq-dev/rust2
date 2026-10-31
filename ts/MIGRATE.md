# Migrating

## Unreleased

- `rt.btreeCollect` takes whether it sorts and the keys' `!ne` after the entries, and
  `rt.btreeCollectKeys` the keys' `PartialOrd::lt` and `!ne`, as in
  `rt.btreeCollect(m, entries, true, Key.eq$cmp_PartialEq)`; neither takes a test for keys
  whose impls are derived or std's. `rt.btreeFromSorted` takes the keys' `!ne` too, and
  what drops a key.
- `rt.cmpTuple` takes the tuple type it compares and what it returns, `rt.Ordering` for
  `Ord::cmp` or `rt.Ordering | undefined` for `PartialOrd::partial_cmp`, as type
  arguments, as in `rt.cmpTuple<[rt.u8, Key], rt.Ordering>([rt.cmp, Key.cmp$cmp_Ord])`.

- An instance's suffix spells each type, trait, and closure by its path, the crate's own
  by its module path, a dependency's under its crate, and std's by its public path
  without a module its item repeats; `&` as `ref`, `&mut` as `mut`, and a slice, array,
  or tuple with its length; and a function's nested item or async block by where it is.
  No name is numbered `$2` on any more: `Fault.fmt$Display` is `Fault.fmt$fmt_Display`,
  `Group.encode$Encode_Version` is `Group.encode$coding_encode_Encode_lite_version_Version`,
  `read$u8s` is `read$mut_ref_slice_u8`, `through$2` is `through$u16s`, and
  `blocksFuture$2` is `blocksFuture$closure0`. A type declared in a function has its
  module suffixed alike, `copy.Pair$run.ts`. A trait object's class is `of$` and its
  type, `Shape.of$dyns_Square` for `Shape.Square`, or `ofPlace$` for one of a place,
  rather than `Square$Place`. A binding's export of a trait's function names the trait by
  its path too: `eq$cmp_PartialEq`, `cmp$cmp_Ord`, `deref$ops_Deref`, and
  `from$convert_From_Vec_u8`. A caller takes the new names from the output.
- `rt.Waker` has `data` and `vtable`, its identity, in place of `willWake`, which is
  `rt.willWake(a, b)`: a caller's own waker gives as `data` what each of its clones
  shares, such as the task it wakes, and as `vtable` an object of its kind of waker.
- A `start..` is an `rt.RangeFrom`, an iterator as `rt.Range` is, rather than
  `{ start }`: a caller passes one as `new rt.RangeFrom(start, rt.u64Step)`, with its
  integer type's `Step`, or `rt.charStep`, and none for other types. A binding taking one
  still reads its `start`. An `rt.Range` or `rt.RangeInclusive` of `char`s takes
  `rt.charStep` too, to step past the surrogates. One of unsigned `NonZero`s takes
  `rt.nonZeroStep` of its integer's `Step` and maximum, as in
  `new rt.RangeInclusive(lo, hi, rt.nonZeroStep(rt.u8Step, 255))`.
- A closure, a `fn` pointer, a `dyn Fn`, or a function item as a value returns
  `undefined` for `()`, not `void`, as in `(a0: rt.u8) => undefined`: a caller passes such
  a parameter a function typed `=> undefined`.
- An instance of a generic std function, such as `core/option.ts`'s, is named for a hash
  of its text, `map$1cf8u0y`, rather than for its type arguments and the closure of one
  of the instances merged into it, `map$Option_u64_u64_rescale_closure0`. A caller that
  imports one by name takes the new name from the output.
- `rt.hashInt` is one function per width, as std writes each integer: `rt.hash8` for a
  `u8`, `i8`, or `bool`, `rt.hash16`, `rt.hash32` for a `char`, `usize`, or `isize` too,
  `rt.hash64`, and `rt.hash128`. `rt.Hasher` is an interface, and `rt.hashFinish` of a
  `DefaultHasher` is std's SipHash-1-3 of what was written, a whole `rt.u64`.
- A `&mut` to an `Arc` or a `Waker`, pinned or boxed too, is an `rt.Place` of it, as a `&mut`
  to a number is, rather than the object: a caller passes one as `{ v: arc }` and reads
  the `Arc` it holds after the call from `v`.
- `bytes::BytesMut` can't be bound: translation maps it, as an `rt.BytesMut`. Delete its
  `[bind]` entry and the file it names.
- A binding's path names a dependency the package renames (`foo = { package = "bar" }`)
  as the package does, `foo::Item`, no longer by its crate name, `bar::Item`.
- A panic aborts, as on wasm32: the output throws `rt.Panic` without running any cleanup,
  so nothing drops as it unwinds, and what the translation held is undefined after one.
  A caller that catches `rt.Panic` must not use the translation's values again.
- Runtime functions lost the droppers only a panic used: `rt.resultUnwrap`,
  `rt.resultExpect`, `rt.resultUnwrapErr`, `rt.vecInsert`, `rt.tupleClone`, `rt.onceSet`,
  `rt.onceGetOrInit`, `rt.hashInsert`, `rt.btreeInsert`, `rt.entryOrInsertWith`,
  `rt.entryOrInsertWithMut`, `rt.iterInspect`, `rt.iterFold`, and `rt.iterCollectResult`
  take one fewer, `hashExtend` and `hashExtendKeys` no longer take `fresh`, an `rt.Iter`'s
  `fold` and `rfold` no longer take `dropAcc`, `iterToArray` no longer takes `"place"`,
  and `rt.vecClone` is gone: clone an array with `map`.
- `Mutex::lock`'s result is an `rt.Result` of the guard, always `Ok`, rather than the
  guard itself.
- A `Weak` and a `MutexGuard` drop: an `rt.Weak` uncounts itself from its `rt.Arc` and a
  guard unlocks its `rt.Mutex`. A struct holding either is a class made with `new` rather
  than an object literal, and a caller drops an `rt.Weak` it holds with `rt.drop`.
- A struct's class takes its fields as arguments in declaration order, `new C(a, b)`,
  rather than as one object, `new C({ a, b })`.
- `tsc` checks a binding of a function returning `()` against `=> undefined`, not
  `=> void`: type its export `: undefined`.
- `rt.ordMax`, `rt.ordMin`, and `rt.sliceSortByKey` take a `PartialOrd::lt` rather than a
  `partial_cmp`, and `rt.ordClamp` takes `le`, `lt`, and `gt`; `rt.cmpLt` makes one from a
  `partial_cmp`. `rt.sliceSortBy` takes only an `Ord::cmp`-like comparator: sort by an
  `lt` with `rt.sliceSort`.
- A trait object in a function's suffix is named for its trait and the types it binds:
  `show$dynstdfmtDebug` is `show$dyn_Debug`, and `poll$dyn_Future_u64` is a
  `dyn Future<Output = u64>`'s.
- A runtime consumer that takes an iterator by value is told whether it drops rather than
  given `rt.drop`: `iterFold`, `iterForEach`, `iterSum`, `iterProduct`, `iterMax`,
  `iterMin`, `iterMaxByKey`, `iterMinByKey`, and `durationSum` take `true`, and
  `ownedCount` and `ownedLast` take only what drops an item. `iterToArray`,
  `iterToByteVec`, `iterExtend`, `iterExtendBytes`, `dequeExtend`, `hashExtend`,
  `hashExtendKeys`, `btreeExtend`, and `btreeExtendKeys` take an `rt.Take`, `"next"` or
  `"fold"`, for how std's reads it; `iterToArray` takes `"place"` where its `inPlace` was
  `true`.
- The runtime's `iterMax`, `iterMin`, `iterMaxByKey`, and `iterMinByKey` take a
  `PartialOrd::lt` after the iterator and any key function, for items or keys of any type:
  `rt.iterMax(it, rt.cmpLt(rt.cmpStr))`, or of integers, `rt.iterMax(it, rt.lt)`.
- `rt.FromUtf8Error` holds its `rt.Utf8Error` as std's does: `e.validUpTo` and
  `e.errorLen` are `e.error.validUpTo` and `e.error.errorLen`.
- `ts.toml` no longer takes `ignore-macros`: `tracing`'s events are left out without it,
  and no other macro is. Delete the key.
- `bytes::Bytes` and `rand::rngs::ThreadRng` can't be bound: translation maps them, a
  `Bytes` as an `rt.ByteSlice`. Delete their `[bind]` entries and the files they name.
- The runtime no longer exports `ownedNth`, `ownedNthBack`, `ownedSkip`, `ownedTake`, or
  `ownedStepBy`: `iterNth`, `iterNthBack`, `iterSkip`, `iterTake`, and `iterStepBy` take
  what drops an item last. `rt.Iter`'s `nthItem` and `nthBackItem` are `nth` and
  `nthBack`, which return `rt.DONE` past the end.
- `bool::then` and `then_some` lower from core's MIR into `core/bool.ts`, so the runtime
  no longer exports `boolThen` or `thenSome`.
- Translation builds std from source for `wasm32-unknown-unknown`, so the toolchain needs
  its `rust-src` component, as the flake's has, and the first run compiles std.
- `Option`'s and `Result`'s combinators lower from core's MIR into `core/option.ts` and
  `core/result.ts`, so the runtime no longer exports `optionMap`, `optionMapOr`,
  `optionUnwrapOr`, `optionOkOr`, `resultOk`, `resultErr`, `resultMap`, `resultMapErr`,
  `resultAndThen`, `resultInspectErr`, or `resultUnwrapOr`.
- `--census` starts from the config's roots, as translation does. For the census of every
  item, census with a config that names no roots: `roots = []`.
- A struct is a plain object typed by an interface, unless it may drop: build one as a
  literal, `{ subscribe, sequence, frameStart: 0 }` rather than
  `new Group({ subscribe, sequence, frameStart: 0 })`.
- Each type has a module of its own beside its Rust module's, holding the type and its
  functions: `coding/decode.Decoder.ts` for `coding::decode::Decoder`. Import it whole,
  `import * as Decoder from "./coding/decode.Decoder.ts"`, and name the type
  `Decoder.Decoder`.
- A function of a type takes `self` first: `dec.slice(4)` is `Decoder.slice(dec, 4)`,
  `group.encode(w, v)` is `Group.encode$Encode_Version(group, w, v)`, and
  `Decoder.new(buf, form)` keeps its spelling, from the type's module.
- An enum's functions move from its companion object to its module:
  `DecodeError.fmt$Display(err, f)` keeps its spelling, after
  `import * as DecodeError from "./coding/decode.DecodeError.ts"`.
- A function's suffix no longer depends on what else is reached. A trait impl's function
  always names its trait, with the trait's type arguments, and a generic instance or a
  function of a generic type always names its type arguments: `Log.clone` is
  `Log.clone$Clone`, `Version.encode` is `Version.encode$Encode_Version`, and
  `Wrap.fmt$Pair` is `Wrap.fmt$Debug_Pair`. A closure type in a suffix is named for where
  it is defined, `decode_closure0`.
- `rt.sliceReverse` is gone: call the array's own `reverse`.
