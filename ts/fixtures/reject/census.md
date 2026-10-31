# Census of `reject-fixture`

- Refusals: 178
- Function instances: 233
- Generic functions no instance reached, lowered over their type parameters: 11

## By construct

| Count | Construct | First |
| ---: | --- | --- |
| 45 | a raw pointer to `_` in `_`, which the crate may put another object in place of | fixtures/reject/src/lib.rs:623:10 |
| 18 | unsupported call to `_` | fixtures/reject/src/lib.rs:167:8 |
| 17 | a raw pointer to `_` in a variable assigned again while it may point there | fixtures/reject/src/lib.rs:631:10 |
| 12 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | fixtures/reject/src/lib.rs:1352:33 |
| 4 | a variant of `_`, which is not supported: either is the same JS string | fixtures/reject/src/lib.rs:344:2 |
| 3 | `_` with a mutable reference to a value JS copies, which its mapping would copy | fixtures/reject/src/lib.rs:529:12 |
| 3 | a copy of a closure that changes what it captured | fixtures/reject/src/lib.rs:237:14 |
| 3 | an assignment through a reference to `_` | fixtures/reject/src/lib.rs:90:2 |
| 3 | replacing the value behind a reference to `_` | fixtures/reject/src/lib.rs:98:2 |
| 3 | unsupported type `_` | fixtures/reject/src/lib.rs:7:14 |
| 2 | `Extend` of `_` by `_`, whose `_` of `_` is not supported yet | fixtures/reject/src/lib.rs:1282:31 |
| 2 | `_` is not supported: its `None` and `Some` would both be `rt.None` | fixtures/reject/src/lib.rs:16:15 |
| 2 | `_` of `_`, whose `None` is `rt.None`, which is not supported yet | fixtures/reject/src/lib.rs:37:4 |
| 2 | `_` of `_`, whose `clone_from` is its own, which is not supported yet | fixtures/reject/src/lib.rs:21:4 |
| 2 | `_`, a trait object of another crate's trait, which is not supported yet | fixtures/reject/src/lib.rs:11:16 |
| 2 | `_`, a trait object of the generic trait `_`, which is not supported yet | fixtures/reject/src/lib.rs:472:23 |
| 2 | a coercion to `_`, which is not supported yet | fixtures/reject/src/lib.rs:463:40 |
| 2 | a raw pointer to a place of `()`, which has no address of its own | fixtures/reject/src/lib.rs:743:15 |
| 2 | an iterator of the crate implementing `_`, which is not supported yet | fixtures/reject/src/lib.rs:152:1 |
| 2 | overriding `_`, which std's adaptors call in place of `next` or beside it | fixtures/reject/src/lib.rs:132:2 |
| 2 | passing `_`, which drops, to `_`, which is not supported yet | fixtures/reject/src/lib.rs:339:4 |
| 2 | unsupported type `_` from another crate | fixtures/reject/src/lib.rs:446:32 |
| 1 | `Buf::copy_to_bytes` of `_`, a value JS copies, which is not supported | fixtures/reject/src/lib.rs:776:2 |
| 1 | `BufMut::put` of `_` into a `BytesMut`, which picks the methods it calls by a capacity that depends on whether the other handles of its storage dropped, which is not supported | fixtures/reject/src/lib.rs:402:2 |
| 1 | `Bytes::from_owner` of `_`, which would drop with the last clone of what it makes, which is not supported | fixtures/reject/src/lib.rs:387:2 |
| 1 | `Pointer` formatting, as in `{:p}`, is not supported | fixtures/reject/src/lib.rs:56:2 |
| 1 | `_` is not supported: its `Pending` and `Ready` would both be `rt.OuterPending` | fixtures/reject/src/lib.rs:79:20 |
| 1 | `_` is not supported: its field `_` is `_`, typed `_`, whose `None` would be `rt.None` as `_` | fixtures/reject/src/lib.rs:31:2 |
| 1 | `_` is not supported: its field `_` is `_`, typed `_`, whose `Pending` would be `rt.OuterPending` as `_` | fixtures/reject/src/lib.rs:510:2 |
| 1 | `_` of `_` by `_`, which is not supported yet | fixtures/reject/src/lib.rs:266:6 |
| 1 | `_` of `_`, which is not supported yet | fixtures/reject/src/lib.rs:553:4 |
| 1 | `_` of a reference to an array of bytes it is generic over, which its mapping would take as the array, not as a slice of it | fixtures/reject/src/lib.rs:227:12 |
| 1 | `_` of a value JS copies, which is no path | fixtures/reject/src/lib.rs:1652:2 |
| 1 | `_`, which is not supported yet | fixtures/reject/src/lib.rs:274:9 |
| 1 | `_`, which reads a capacity that depends on whether the other handles of its storage dropped, which is not supported | fixtures/reject/src/lib.rs:396:6 |
| 1 | `collect` of `_`, which sorts by `_`'s own `Ord`, of entries that drop, which is not supported yet | fixtures/reject/src/lib.rs:970:22 |
| 1 | `fmt::Arguments` with a placeholder, or not made in place, written to `_`'s own `fmt::Write`, which is not supported yet: std splits what a placeholder writes into `write_str` calls the runtime does not make | fixtures/reject/src/lib.rs:62:10 |
| 1 | a `collect` in place of `_`, a zip std reads by index, holding values that drop, which is not supported yet | fixtures/reject/src/lib.rs:216:40 |
| 1 | a `skip` of `_`, a zip std reads by index, of items that drop, which is not supported yet | fixtures/reject/src/lib.rs:209:8 |
| 1 | a cast from `_` to `_` | fixtures/reject/src/lib.rs:75:2 |
| 1 | a cast from a pointer to `_` to one to `_`, which reads the bytes of one type as another | fixtures/reject/src/lib.rs:570:27 |
| 1 | a coercion to `_` | fixtures/reject/src/lib.rs:304:2 |
| 1 | a constant of type `_`, a pointer into memory translation can't name | fixtures/reject/src/lib.rs:793:3 |
| 1 | a dangling pointer to `_`, a value of no bytes made of more than structs and tuples | fixtures/reject/src/lib.rs:752:11 |
| 1 | a future of `_`, which can be `rt.Pending`, as the future's own `Pending` is | fixtures/reject/src/lib.rs:84:34 |
| 1 | a mutable reference into a `Result`'s payload, passed on or kept | fixtures/reject/src/lib.rs:375:12 |
| 1 | a mutable static, which any code may write | fixtures/reject/src/lib.rs:68:11 |
| 1 | a pointer's address, which JS has no counterpart of | fixtures/reject/src/lib.rs:575:17 |
| 1 | a printable type must implement `Display` or `Debug` | fixtures/reject/src/lib.rs:71:1 |
| 1 | a raw pointer to `_` behind a reference, which is the object and not where it is kept | fixtures/reject/src/lib.rs:592:10 |
| 1 | a raw pointer to `_` in an element, a payload, or a union field, which is not supported yet | fixtures/reject/src/lib.rs:694:10 |
| 1 | a scalar constant of type `_` | fixtures/reject/src/lib.rs:8:2 |
| 1 | a trait object of `_`, which is not supported yet | fixtures/reject/src/lib.rs:489:26 |
| 1 | a zero-sized constant of type `_` | fixtures/reject/src/lib.rs:50:2 |
| 1 | an upcast from `_` to `_`, which is not supported yet | fixtures/reject/src/lib.rs:547:32 |
| 1 | arithmetic on `_` | fixtures/reject/src/lib.rs:8:2 |
| 1 | comparing `_` as a key, which is not supported yet | fixtures/reject/src/lib.rs:248:75 |
| 1 | comparing pointers to `_` in an atomic, which JS has no address of | fixtures/reject/src/lib.rs:792:15 |
| 1 | comparing pointers to `_`, each a place JS makes of its own | fixtures/reject/src/lib.rs:611:2 |
| 1 | comparing pointers to `_`, which has no bytes and so no address of its own | fixtures/reject/src/lib.rs:743:70 |
| 1 | constructing `_` | fixtures/reject/src/lib.rs:362:53 |
| 1 | hashing `_` as a key, which is not supported yet | fixtures/reject/src/lib.rs:248:75 |
| 1 | hashing `_` through `_`'s own `Hash::hash_slice`, which is not supported yet | fixtures/reject/src/lib.rs:323:16 |
| 1 | passing `_`, a closure that owns a value that drops, to `_`, which is not supported yet | fixtures/reject/src/lib.rs:332:13 |
| 1 | union `_`, whose fields overlap in `_` and `f64`, which JS keeps apart | fixtures/reject/src/lib.rs:597:1 |

## By API

The type or module each refused type or path belongs to.

| Count | API | First |
| ---: | --- | --- |
| 31 | `std::cell::UnsafeCell` | fixtures/reject/src/lib.rs:702:15 |
| 24 | `(u32, u32)` | fixtures/reject/src/lib.rs:623:10 |
| 6 | `f16` | fixtures/reject/src/lib.rs:7:14 |
| 6 | `std::borrow::Cow` | fixtures/reject/src/lib.rs:344:2 |
| 5 | `Gap` | fixtures/reject/src/lib.rs:923:10 |
| 5 | `Option` | fixtures/reject/src/lib.rs:16:15 |
| 5 | `u32` | fixtures/reject/src/lib.rs:570:27 |
| 5 | `u8` | fixtures/reject/src/lib.rs:266:6 |
| 4 | `Iterator` | fixtures/reject/src/lib.rs:37:4 |
| 4 | `std::pin::Pin` | fixtures/reject/src/lib.rs:98:2 |
| 3 | `std::vec::Vec` | fixtures/reject/src/lib.rs:323:16 |
| 3 | `{closure}` | fixtures/reject/src/lib.rs:332:13 |
| 2 | `(u16, u16)` | fixtures/reject/src/lib.rs:1191:31 |
| 2 | `InArray` | fixtures/reject/src/lib.rs:1317:9 |
| 2 | `InField` | fixtures/reject/src/lib.rs:1333:9 |
| 2 | `InNested` | fixtures/reject/src/lib.rs:1325:9 |
| 2 | `InPayload` | fixtures/reject/src/lib.rs:1329:9 |
| 2 | `InTuple` | fixtures/reject/src/lib.rs:1321:9 |
| 2 | `Poll` | fixtures/reject/src/lib.rs:79:20 |
| 2 | `Reused` | fixtures/reject/src/lib.rs:21:4 |
| 2 | `Scaled` | fixtures/reject/src/lib.rs:472:23 |
| 2 | `Tally` | fixtures/reject/src/lib.rs:1282:31 |
| 2 | `dyn Scaled` | fixtures/reject/src/lib.rs:472:23 |
| 2 | `dyn std::any::Any` | fixtures/reject/src/lib.rs:547:32 |
| 2 | `dyn std::iter::Iterator` | fixtures/reject/src/lib.rs:11:16 |
| 2 | `slice` | fixtures/reject/src/lib.rs:21:4 |
| 2 | `std::any::TypeId` | fixtures/reject/src/lib.rs:553:2 |
| 2 | `std::collections::BTreeSet` | fixtures/reject/src/lib.rs:248:75 |
| 2 | `std::mem::MaybeUninit` | fixtures/reject/src/lib.rs:937:19 |
| 2 | `std::ops::Range` | fixtures/reject/src/lib.rs:167:8 |
| 2 | `std::option::Option` | fixtures/reject/src/lib.rs:227:12 |
| 2 | `std::std` | fixtures/reject/src/lib.rs:582:11 |
| 2 | `std::sync::Arc` | fixtures/reject/src/lib.rs:304:2 |
| 2 | `std::sync::MutexGuard` | fixtures/reject/src/lib.rs:90:2 |
| 2 | `std::vec::IntoIter` | fixtures/reject/src/lib.rs:442:17 |
| 1 | `(Tally,)` | fixtures/reject/src/lib.rs:1287:29 |
| 1 | `(std::vec::Vec, Tally)` | fixtures/reject/src/lib.rs:1282:31 |
| 1 | `(u16, ByKeyReplace)` | fixtures/reject/src/lib.rs:1636:55 |
| 1 | `(u16, ByKeySet)` | fixtures/reject/src/lib.rs:1628:51 |
| 1 | `(u16, ByKeyTake)` | fixtures/reject/src/lib.rs:1643:52 |
| 1 | `(u16, ByRefReplace)` | fixtures/reject/src/lib.rs:1612:32 |
| 1 | `(u16, ByRefTake)` | fixtures/reject/src/lib.rs:1620:32 |
| 1 | `(u8, u8)` | fixtures/reject/src/lib.rs:1282:31 |
| 1 | `(u8,)` | fixtures/reject/src/lib.rs:1287:29 |
| 1 | `Aliased` | fixtures/reject/src/lib.rs:1135:29 |
| 1 | `Batched` | fixtures/reject/src/lib.rs:323:16 |
| 1 | `Bits` | fixtures/reject/src/lib.rs:597:1 |
| 1 | `Blank` | fixtures/reject/src/lib.rs:776:2 |
| 1 | `ByBox` | fixtures/reject/src/lib.rs:1519:35 |
| 1 | `ByClosure` | fixtures/reject/src/lib.rs:1534:35 |
| 1 | `ByExchanged` | fixtures/reject/src/lib.rs:1580:30 |
| 1 | `ByInstance` | fixtures/reject/src/lib.rs:1560:35 |
| 1 | `ByKeyReplace` | fixtures/reject/src/lib.rs:1636:55 |
| 1 | `ByKeySet` | fixtures/reject/src/lib.rs:1628:51 |
| 1 | `ByKeyTake` | fixtures/reject/src/lib.rs:1643:52 |
| 1 | `ByObject` | fixtures/reject/src/lib.rs:1547:35 |
| 1 | `ByRead` | fixtures/reject/src/lib.rs:1470:35 |
| 1 | `ByRefReplace` | fixtures/reject/src/lib.rs:1612:32 |
| 1 | `ByRefTake` | fixtures/reject/src/lib.rs:1620:32 |
| 1 | `ByReplace` | fixtures/reject/src/lib.rs:1508:35 |
| 1 | `BySwap` | fixtures/reject/src/lib.rs:1496:35 |
| 1 | `BySwapped` | fixtures/reject/src/lib.rs:1569:76 |
| 1 | `ByWrite` | fixtures/reject/src/lib.rs:1483:35 |
| 1 | `BytesMut` | fixtures/reject/src/lib.rs:396:6 |
| 1 | `Called` | fixtures/reject/src/lib.rs:1101:28 |
| 1 | `Copied` | fixtures/reject/src/lib.rs:1191:31 |
| 1 | `Core` | fixtures/reject/src/lib.rs:1746:18 |
| 1 | `Counted` | fixtures/reject/src/lib.rs:402:2 |
| 1 | `Counter` | fixtures/reject/src/lib.rs:1721:19 |
| 1 | `DoubleEndedIterator` | fixtures/reject/src/lib.rs:152:1 |
| 1 | `Entry` | fixtures/reject/src/lib.rs:332:13 |
| 1 | `Escaped` | fixtures/reject/src/lib.rs:1087:43 |
| 1 | `FusedIterator` | fixtures/reject/src/lib.rs:200:1 |
| 1 | `Fused` | fixtures/reject/src/lib.rs:743:70 |
| 1 | `HashMap` | fixtures/reject/src/lib.rs:266:6 |
| 1 | `Hasher` | fixtures/reject/src/lib.rs:274:9 |
| 1 | `Hits` | fixtures/reject/src/lib.rs:735:21 |
| 1 | `InGeneric` | fixtures/reject/src/lib.rs:1337:9 |
| 1 | `InWritten` | fixtures/reject/src/lib.rs:1341:9 |
| 1 | `Lane` | fixtures/reject/src/lib.rs:887:19 |
| 1 | `Loaded` | fixtures/reject/src/lib.rs:1157:29 |
| 1 | `Looped` | fixtures/reject/src/lib.rs:1125:26 |
| 1 | `Mapped` | fixtures/reject/src/lib.rs:1202:39 |
| 1 | `Maybe` | fixtures/reject/src/lib.rs:31:2 |
| 1 | `Onced` | fixtures/reject/src/lib.rs:1248:46 |
| 1 | `Only` | fixtures/reject/src/lib.rs:752:11 |
| 1 | `Owned` | fixtures/reject/src/lib.rs:1672:19 |
| 1 | `Passed` | fixtures/reject/src/lib.rs:1704:10 |
| 1 | `Picked` | fixtures/reject/src/lib.rs:1804:25 |
| 1 | `Plain` | fixtures/reject/src/lib.rs:841:10 |
| 1 | `Pushed` | fixtures/reject/src/lib.rs:1263:22 |
| 1 | `Repeated` | fixtures/reject/src/lib.rs:1236:18 |
| 1 | `Rewritten` | fixtures/reject/src/lib.rs:1145:47 |
| 1 | `Sink` | fixtures/reject/src/lib.rs:62:10 |
| 1 | `Span2` | fixtures/reject/src/lib.rs:623:10 |
| 1 | `Stored` | fixtures/reject/src/lib.rs:1688:13 |
| 1 | `String` | fixtures/reject/src/lib.rs:799:4 |
| 1 | `T \| undefined` | fixtures/reject/src/lib.rs:31:2 |
| 1 | `Tail` | fixtures/reject/src/lib.rs:499:30 |
| 1 | `Target` | fixtures/reject/src/lib.rs:1775:10 |
| 1 | `Track` | fixtures/reject/src/lib.rs:867:19 |
| 1 | `Waiting` | fixtures/reject/src/lib.rs:510:2 |
| 1 | `Window` | fixtures/reject/src/lib.rs:684:2 |
| 1 | `[u8; 4]` | fixtures/reject/src/lib.rs:489:26 |
| 1 | `dyn Keyed` | fixtures/reject/src/lib.rs:547:32 |
| 1 | `poll` | fixtures/reject/src/lib.rs:510:2 |
| 1 | `rt.Poll` | fixtures/reject/src/lib.rs:510:2 |
| 1 | `std::any::Any` | fixtures/reject/src/lib.rs:553:4 |
| 1 | `std::cell::Cell` | fixtures/reject/src/lib.rs:970:22 |
| 1 | `std::collections::BTreeMap` | fixtures/reject/src/lib.rs:970:22 |
| 1 | `std::collections::HashMap` | fixtures/reject/src/lib.rs:266:6 |
| 1 | `std::iter::Map` | fixtures/reject/src/lib.rs:216:40 |
| 1 | `std::iter::Zip` | fixtures/reject/src/lib.rs:209:8 |
| 1 | `std::iter` | fixtures/reject/src/lib.rs:1250:22 |
| 1 | `std::net::AddrParseError` | fixtures/reject/src/lib.rs:446:32 |
| 1 | `std::net::Ipv4Addr` | fixtures/reject/src/lib.rs:446:32 |
| 1 | `std::num` | fixtures/reject/src/lib.rs:1292:4 |
| 1 | `std::ops::DerefMut` | fixtures/reject/src/lib.rs:1652:2 |
| 1 | `std::ops::IndexMut` | fixtures/reject/src/lib.rs:1658:3 |
| 1 | `std::ops::RangeFrom` | fixtures/reject/src/lib.rs:999:32 |
| 1 | `std::ops::RangeTo` | fixtures/reject/src/lib.rs:999:11 |
| 1 | `std::slice::IterMut` | fixtures/reject/src/lib.rs:781:15 |
| 1 | `std::std::Layout` | fixtures/reject/src/lib.rs:580:15 |
| 1 | `std::str` | fixtures/reject/src/lib.rs:447:4 |
| 1 | `std::sync::Mutex` | fixtures/reject/src/lib.rs:529:12 |
| 1 | `std::sync::Weak` | fixtures/reject/src/lib.rs:463:40 |
| 1 | `std::task::Poll` | fixtures/reject/src/lib.rs:84:34 |
| 1 | `str` | fixtures/reject/src/lib.rs:339:4 |
| 1 | `u64` | fixtures/reject/src/lib.rs:75:2 |
| 1 | `value` | fixtures/reject/src/lib.rs:31:2 |
| 1 | `{async closure}` | fixtures/reject/src/lib.rs:50:2 |

## By type or path

| Count | Construct | Type or path | First |
| ---: | --- | --- | --- |
| 14 | a raw pointer to `_` in a variable assigned again while it may point there | `(u32, u32)` | fixtures/reject/src/lib.rs:631:10 |
| 4 | a variant of `_`, which is not supported: either is the same JS string | `std::borrow::Cow` | fixtures/reject/src/lib.rs:344:2 |
| 3 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u32, u32)`, `Gap` | fixtures/reject/src/lib.rs:923:10 |
| 3 | unsupported type `_` | `f16` | fixtures/reject/src/lib.rs:7:14 |
| 2 | `_` is not supported: its `None` and `Some` would both be `rt.None` | `Option` | fixtures/reject/src/lib.rs:16:15 |
| 2 | `_` of `_`, whose `None` is `rt.None`, which is not supported yet | `Iterator::scan`, `Option` | fixtures/reject/src/lib.rs:37:4 |
| 2 | `_`, a trait object of another crate's trait, which is not supported yet | `dyn std::iter::Iterator` | fixtures/reject/src/lib.rs:11:16 |
| 2 | `_`, a trait object of the generic trait `_`, which is not supported yet | `dyn Scaled`, `Scaled` | fixtures/reject/src/lib.rs:472:23 |
| 2 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::mem::MaybeUninit`, `Gap` | fixtures/reject/src/lib.rs:937:19 |
| 2 | a raw pointer to `_` in a variable assigned again while it may point there | `std::cell::UnsafeCell` | fixtures/reject/src/lib.rs:711:15 |
| 2 | an assignment through a reference to `_` | `std::pin::Pin` | fixtures/reject/src/lib.rs:114:2 |
| 2 | replacing the value behind a reference to `_` | `std::pin::Pin` | fixtures/reject/src/lib.rs:98:2 |
| 2 | unsupported call to `_` | `<std::vec::IntoIter as std::iter::Iterator>::sum` | fixtures/reject/src/lib.rs:442:17 |
| 1 | `Buf::copy_to_bytes` of `_`, a value JS copies, which is not supported | `Blank` | fixtures/reject/src/lib.rs:776:2 |
| 1 | `BufMut::put` of `_` into a `BytesMut`, which picks the methods it calls by a capacity that depends on whether the other handles of its storage dropped, which is not supported | `Counted` | fixtures/reject/src/lib.rs:402:2 |
| 1 | `Bytes::from_owner` of `_`, which would drop with the last clone of what it makes, which is not supported | `std::sync::Arc` | fixtures/reject/src/lib.rs:387:2 |
| 1 | `Extend` of `_` by `_`, whose `_` of `_` is not supported yet | `(Tally,)`, `(u8,)`, `Tally`, `u8` | fixtures/reject/src/lib.rs:1287:29 |
| 1 | `Extend` of `_` by `_`, whose `_` of `_` is not supported yet | `(std::vec::Vec, Tally)`, `(u8, u8)`, `Tally`, `u8` | fixtures/reject/src/lib.rs:1282:31 |
| 1 | `_` is not supported: its `Pending` and `Ready` would both be `rt.OuterPending` | `Poll` | fixtures/reject/src/lib.rs:79:20 |
| 1 | `_` is not supported: its field `_` is `_`, typed `_`, whose `None` would be `rt.None` as `_` | `Maybe`, `value`, `Option`, `T \| undefined`, `Option` | fixtures/reject/src/lib.rs:31:2 |
| 1 | `_` is not supported: its field `_` is `_`, typed `_`, whose `Pending` would be `rt.OuterPending` as `_` | `Waiting`, `poll`, `Poll`, `rt.Poll`, `Poll` | fixtures/reject/src/lib.rs:510:2 |
| 1 | `_` of `_` by `_`, which is not supported yet | `HashMap::contains_key`, `std::collections::HashMap`, `u8` | fixtures/reject/src/lib.rs:266:6 |
| 1 | `_` of `_`, which is not supported yet | `std::any::Any::type_id`, `dyn std::any::Any` | fixtures/reject/src/lib.rs:553:4 |
| 1 | `_` of `_`, whose `clone_from` is its own, which is not supported yet | `slice::clone_from_slice`, `Reused` | fixtures/reject/src/lib.rs:299:4 |
| 1 | `_` of `_`, whose `clone_from` is its own, which is not supported yet | `slice::fill`, `Reused` | fixtures/reject/src/lib.rs:21:4 |
| 1 | `_` of a reference to an array of bytes it is generic over, which its mapping would take as the array, not as a slice of it | `std::option::Option::copied` | fixtures/reject/src/lib.rs:227:12 |
| 1 | `_` of a value JS copies, which is no path | `std::ops::DerefMut::deref_mut` | fixtures/reject/src/lib.rs:1652:2 |
| 1 | `_` with a mutable reference to a value JS copies, which its mapping would copy | `std::iter::once` | fixtures/reject/src/lib.rs:1250:22 |
| 1 | `_` with a mutable reference to a value JS copies, which its mapping would copy | `std::ops::IndexMut::index_mut` | fixtures/reject/src/lib.rs:1658:3 |
| 1 | `_` with a mutable reference to a value JS copies, which its mapping would copy | `std::sync::Mutex::get_mut` | fixtures/reject/src/lib.rs:529:12 |
| 1 | `_`, which is not supported yet | `Hasher::write` | fixtures/reject/src/lib.rs:274:9 |
| 1 | `_`, which reads a capacity that depends on whether the other handles of its storage dropped, which is not supported | `BytesMut::capacity` | fixtures/reject/src/lib.rs:396:6 |
| 1 | `collect` of `_`, which sorts by `_`'s own `Ord`, of entries that drop, which is not supported yet | `std::collections::BTreeMap`, `std::cell::Cell` | fixtures/reject/src/lib.rs:970:22 |
| 1 | `fmt::Arguments` with a placeholder, or not made in place, written to `_`'s own `fmt::Write`, which is not supported yet: std splits what a placeholder writes into `write_str` calls the runtime does not make | `Sink` | fixtures/reject/src/lib.rs:62:10 |
| 1 | a `collect` in place of `_`, a zip std reads by index, holding values that drop, which is not supported yet | `std::iter::Map` | fixtures/reject/src/lib.rs:216:40 |
| 1 | a `skip` of `_`, a zip std reads by index, of items that drop, which is not supported yet | `std::iter::Zip` | fixtures/reject/src/lib.rs:209:8 |
| 1 | a cast from `_` to `_` | `u64`, `f16` | fixtures/reject/src/lib.rs:75:2 |
| 1 | a cast from a pointer to `_` to one to `_`, which reads the bytes of one type as another | `u32`, `u8` | fixtures/reject/src/lib.rs:570:27 |
| 1 | a coercion to `_` | `std::sync::Arc` | fixtures/reject/src/lib.rs:304:2 |
| 1 | a coercion to `_`, which is not supported yet | `&Tail` | fixtures/reject/src/lib.rs:499:30 |
| 1 | a coercion to `_`, which is not supported yet | `std::sync::Weak` | fixtures/reject/src/lib.rs:463:40 |
| 1 | a constant of type `_`, a pointer into memory translation can't name | `&u32` | fixtures/reject/src/lib.rs:793:3 |
| 1 | a dangling pointer to `_`, a value of no bytes made of more than structs and tuples | `Only` | fixtures/reject/src/lib.rs:752:11 |
| 1 | a future of `_`, which can be `rt.Pending`, as the future's own `Pending` is | `std::task::Poll` | fixtures/reject/src/lib.rs:84:34 |
| 1 | a raw pointer to `_` behind a reference, which is the object and not where it is kept | `std::vec::Vec` | fixtures/reject/src/lib.rs:592:10 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u16, u16)`, `Copied` | fixtures/reject/src/lib.rs:1191:31 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u16, u16)`, `Pushed` | fixtures/reject/src/lib.rs:1263:22 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u32, u32)`, `Lane` | fixtures/reject/src/lib.rs:887:19 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u32, u32)`, `Plain` | fixtures/reject/src/lib.rs:841:10 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u32, u32)`, `Span2` | fixtures/reject/src/lib.rs:623:10 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u32, u32)`, `Track` | fixtures/reject/src/lib.rs:867:19 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u32, u32)`, `Window` | fixtures/reject/src/lib.rs:684:2 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u32, u32)`, `std::cell::UnsafeCell` | fixtures/reject/src/lib.rs:702:15 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `ByKeyReplace`, `(u16, ByKeyReplace)` | fixtures/reject/src/lib.rs:1636:55 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `ByKeySet`, `(u16, ByKeySet)` | fixtures/reject/src/lib.rs:1628:51 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `ByKeyTake`, `(u16, ByKeyTake)` | fixtures/reject/src/lib.rs:1643:52 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `ByRefReplace`, `(u16, ByRefReplace)` | fixtures/reject/src/lib.rs:1612:32 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `ByRefTake`, `(u16, ByRefTake)` | fixtures/reject/src/lib.rs:1620:32 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Aliased` | fixtures/reject/src/lib.rs:1135:29 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByBox` | fixtures/reject/src/lib.rs:1519:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByClosure` | fixtures/reject/src/lib.rs:1534:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByExchanged` | fixtures/reject/src/lib.rs:1580:30 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByInstance` | fixtures/reject/src/lib.rs:1560:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByObject` | fixtures/reject/src/lib.rs:1547:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByRead` | fixtures/reject/src/lib.rs:1470:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByReplace` | fixtures/reject/src/lib.rs:1508:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `BySwap` | fixtures/reject/src/lib.rs:1496:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `BySwapped` | fixtures/reject/src/lib.rs:1569:76 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByWrite` | fixtures/reject/src/lib.rs:1483:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Called` | fixtures/reject/src/lib.rs:1101:28 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Escaped` | fixtures/reject/src/lib.rs:1087:43 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Hits` | fixtures/reject/src/lib.rs:735:21 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `InArray` | fixtures/reject/src/lib.rs:1317:9 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `InField` | fixtures/reject/src/lib.rs:1333:9 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `InGeneric` | fixtures/reject/src/lib.rs:1337:9 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `InNested` | fixtures/reject/src/lib.rs:1325:9 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `InPayload` | fixtures/reject/src/lib.rs:1329:9 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `InTuple` | fixtures/reject/src/lib.rs:1321:9 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `InWritten` | fixtures/reject/src/lib.rs:1341:9 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Loaded` | fixtures/reject/src/lib.rs:1157:29 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Looped` | fixtures/reject/src/lib.rs:1125:26 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Mapped` | fixtures/reject/src/lib.rs:1202:39 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Onced` | fixtures/reject/src/lib.rs:1248:46 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Repeated` | fixtures/reject/src/lib.rs:1236:18 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Rewritten` | fixtures/reject/src/lib.rs:1145:47 |
| 1 | a raw pointer to `_` in a variable assigned again while it may point there | `Target` | fixtures/reject/src/lib.rs:1775:10 |
| 1 | a raw pointer to `_` in an element, a payload, or a union field, which is not supported yet | `(u32, u32)` | fixtures/reject/src/lib.rs:694:10 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Core` | fixtures/reject/src/lib.rs:1746:18 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Counter` | fixtures/reject/src/lib.rs:1721:19 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `InArray` | fixtures/reject/src/lib.rs:1352:33 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `InField` | fixtures/reject/src/lib.rs:1391:33 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `InNested` | fixtures/reject/src/lib.rs:1370:34 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `InPayload` | fixtures/reject/src/lib.rs:1379:11 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `InTuple` | fixtures/reject/src/lib.rs:1361:33 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Owned` | fixtures/reject/src/lib.rs:1672:19 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Passed` | fixtures/reject/src/lib.rs:1704:10 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Picked` | fixtures/reject/src/lib.rs:1804:25 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Stored` | fixtures/reject/src/lib.rs:1688:13 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `std::cell::UnsafeCell` | fixtures/reject/src/lib.rs:1761:18 |
| 1 | a scalar constant of type `_` | `f16` | fixtures/reject/src/lib.rs:8:2 |
| 1 | a trait object of `_`, which is not supported yet | `[u8; 4]` | fixtures/reject/src/lib.rs:489:26 |
| 1 | a zero-sized constant of type `_` | `{async closure}` | fixtures/reject/src/lib.rs:50:2 |
| 1 | an assignment through a reference to `_` | `std::sync::MutexGuard` | fixtures/reject/src/lib.rs:90:2 |
| 1 | an iterator of the crate implementing `_`, which is not supported yet | `DoubleEndedIterator` | fixtures/reject/src/lib.rs:152:1 |
| 1 | an iterator of the crate implementing `_`, which is not supported yet | `FusedIterator` | fixtures/reject/src/lib.rs:200:1 |
| 1 | an upcast from `_` to `_`, which is not supported yet | `dyn Keyed`, `dyn std::any::Any` | fixtures/reject/src/lib.rs:547:32 |
| 1 | arithmetic on `_` | `f16` | fixtures/reject/src/lib.rs:8:2 |
| 1 | comparing `_` as a key, which is not supported yet | `std::collections::BTreeSet` | fixtures/reject/src/lib.rs:248:75 |
| 1 | comparing pointers to `_` in an atomic, which JS has no address of | `u32` | fixtures/reject/src/lib.rs:792:15 |
| 1 | comparing pointers to `_`, each a place JS makes of its own | `u32` | fixtures/reject/src/lib.rs:611:2 |
| 1 | comparing pointers to `_`, which has no bytes and so no address of its own | `Fused` | fixtures/reject/src/lib.rs:743:70 |
| 1 | constructing `_` | `std::borrow::Cow` | fixtures/reject/src/lib.rs:362:53 |
| 1 | hashing `_` as a key, which is not supported yet | `std::collections::BTreeSet` | fixtures/reject/src/lib.rs:248:75 |
| 1 | hashing `_` through `_`'s own `Hash::hash_slice`, which is not supported yet | `std::vec::Vec`, `Batched` | fixtures/reject/src/lib.rs:323:16 |
| 1 | overriding `_`, which std's adaptors call in place of `next` or beside it | `Iterator::fold` | fixtures/reject/src/lib.rs:132:2 |
| 1 | overriding `_`, which std's adaptors call in place of `next` or beside it | `Iterator::size_hint` | fixtures/reject/src/lib.rs:180:2 |
| 1 | passing `_`, a closure that owns a value that drops, to `_`, which is not supported yet | `{closure}`, `Entry::or_insert_with` | fixtures/reject/src/lib.rs:332:13 |
| 1 | passing `_`, which drops, to `_`, which is not supported yet | `{closure}`, `String::retain` | fixtures/reject/src/lib.rs:799:4 |
| 1 | passing `_`, which drops, to `_`, which is not supported yet | `{closure}`, `str::contains` | fixtures/reject/src/lib.rs:339:4 |
| 1 | replacing the value behind a reference to `_` | `std::sync::MutexGuard` | fixtures/reject/src/lib.rs:119:2 |
| 1 | union `_`, whose fields overlap in `_` and `f64`, which JS keeps apart | `Bits`, `u32` | fixtures/reject/src/lib.rs:597:1 |
| 1 | unsupported call to `_` | `<std::any::TypeId as std::cmp::PartialEq>::eq` | fixtures/reject/src/lib.rs:553:2 |
| 1 | unsupported call to `_` | `<std::ops::RangeFrom as std::ops::RangeBounds>::contains` | fixtures/reject/src/lib.rs:999:32 |
| 1 | unsupported call to `_` | `<std::option::Option as std::cmp::PartialEq>::eq` | fixtures/reject/src/lib.rs:382:2 |
| 1 | unsupported call to `_` | `<std::slice::IterMut as std::iter::Iterator>::is_sorted` | fixtures/reject/src/lib.rs:781:15 |
| 1 | unsupported call to `_` | `<std::vec::Vec as std::clone::Clone>::clone_from` | fixtures/reject/src/lib.rs:901:10 |
| 1 | unsupported call to `_` | `<u8 as std::hash::Hash>::hash` | fixtures/reject/src/lib.rs:314:10 |
| 1 | unsupported call to `_` | `std::any::TypeId::of` | fixtures/reject/src/lib.rs:553:17 |
| 1 | unsupported call to `_` | `std::borrow::Cow::is_borrowed` | fixtures/reject/src/lib.rs:356:2 |
| 1 | unsupported call to `_` | `std::num::is_ascii_octdigit` | fixtures/reject/src/lib.rs:1292:4 |
| 1 | unsupported call to `_` | `std::ops::Range::contains` | fixtures/reject/src/lib.rs:167:8 |
| 1 | unsupported call to `_` | `std::ops::Range::is_empty` | fixtures/reject/src/lib.rs:1004:9 |
| 1 | unsupported call to `_` | `std::ops::RangeTo::contains` | fixtures/reject/src/lib.rs:999:11 |
| 1 | unsupported call to `_` | `std::std::Layout::new` | fixtures/reject/src/lib.rs:580:15 |
| 1 | unsupported call to `_` | `std::std::alloc` | fixtures/reject/src/lib.rs:582:11 |
| 1 | unsupported call to `_` | `std::std::dealloc` | fixtures/reject/src/lib.rs:585:3 |
| 1 | unsupported call to `_` | `std::str::parse` | fixtures/reject/src/lib.rs:447:4 |
| 1 | unsupported type `_` from another crate | `std::net::AddrParseError` | fixtures/reject/src/lib.rs:446:32 |
| 1 | unsupported type `_` from another crate | `std::net::Ipv4Addr` | fixtures/reject/src/lib.rs:446:32 |

## Spans

<details><summary>a raw pointer to `_` in `_`, which the crate may put another object in place of (45)</summary>

- fixtures/reject/src/lib.rs:623:10: a raw pointer to `(u32, u32)` in `Span2`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:684:2: a raw pointer to `(u32, u32)` in `Window`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:702:15: a raw pointer to `(u32, u32)` in `std::cell::UnsafeCell<(u32, u32)>`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:735:21: a raw pointer to `std::cell::UnsafeCell<u32>` in `Hits`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:841:10: a raw pointer to `(u32, u32)` in `Plain`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:867:19: a raw pointer to `(u32, u32)` in `Track`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:887:19: a raw pointer to `(u32, u32)` in `Lane`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:923:10: a raw pointer to `(u32, u32)` in `Gap`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:924:10: a raw pointer to `(u32, u32)` in `Gap`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:925:58: a raw pointer to `(u32, u32)` in `Gap`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:937:19: a raw pointer to `std::mem::MaybeUninit<(u32, u32)>` in `Gap`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:938:19: a raw pointer to `std::mem::MaybeUninit<(u32, u32)>` in `Gap`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1087:43: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Escaped`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1101:28: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Called`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1125:26: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Looped`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1135:29: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Aliased`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1145:47: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Rewritten`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1157:29: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Loaded`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1191:31: a raw pointer to `(u16, u16)` in `Copied`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1202:39: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Mapped`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1236:18: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Repeated`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1248:46: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Onced`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1263:22: a raw pointer to `(u16, u16)` in `Pushed`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1317:9: a raw pointer to `std::cell::UnsafeCell<u16>` in `InArray`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1321:9: a raw pointer to `std::cell::UnsafeCell<u16>` in `InTuple`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1325:9: a raw pointer to `std::cell::UnsafeCell<u16>` in `InNested`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1329:9: a raw pointer to `std::cell::UnsafeCell<u16>` in `InPayload`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1333:9: a raw pointer to `std::cell::UnsafeCell<u16>` in `InField`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1337:9: a raw pointer to `std::cell::UnsafeCell<u16>` in `InGeneric`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1341:9: a raw pointer to `std::cell::UnsafeCell<u16>` in `InWritten`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1470:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByRead`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1483:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByWrite`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1496:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `BySwap`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1508:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByReplace`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1519:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByBox`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1534:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByClosure`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1547:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByObject`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1560:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByInstance`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1569:76: a raw pointer to `std::cell::UnsafeCell<u16>` in `BySwapped`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1580:30: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByExchanged`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1612:32: a raw pointer to `ByRefReplace` in `(u16, ByRefReplace)`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1620:32: a raw pointer to `ByRefTake` in `(u16, ByRefTake)`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1628:51: a raw pointer to `ByKeySet` in `(u16, ByKeySet)`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1636:55: a raw pointer to `ByKeyReplace` in `(u16, ByKeyReplace)`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1643:52: a raw pointer to `ByKeyTake` in `(u16, ByKeyTake)`, which the crate may put another object in place of

</details>

<details><summary>unsupported call to `_` (18)</summary>

- fixtures/reject/src/lib.rs:167:8: unsupported call to `std::ops::Range::<Key>::contains::<Key>`
- fixtures/reject/src/lib.rs:314:10: unsupported call to `<u8 as std::hash::Hash>::hash::<H>`
- fixtures/reject/src/lib.rs:356:2: unsupported call to `std::borrow::Cow::<'_, [u8]>::is_borrowed`
- fixtures/reject/src/lib.rs:382:2: unsupported call to `<std::option::Option<&mut u32> as std::cmp::PartialEq>::eq`
- fixtures/reject/src/lib.rs:442:17: unsupported call to `<std::vec::IntoIter<Part> as std::iter::Iterator>::sum::<std::time::Duration>`
- fixtures/reject/src/lib.rs:442:38: unsupported call to `<std::vec::IntoIter<Part> as std::iter::Iterator>::sum::<u64>`
- fixtures/reject/src/lib.rs:447:4: unsupported call to `core::str::<impl str>::parse::<std::net::Ipv4Addr>`
- fixtures/reject/src/lib.rs:553:2: unsupported call to `<std::any::TypeId as std::cmp::PartialEq>::eq`
- fixtures/reject/src/lib.rs:553:17: unsupported call to `std::any::TypeId::of::<u8>`
- fixtures/reject/src/lib.rs:580:15: unsupported call to `std::alloc::Layout::new::<u8>`
- fixtures/reject/src/lib.rs:582:11: unsupported call to `std::alloc::alloc`
- fixtures/reject/src/lib.rs:585:3: unsupported call to `std::alloc::dealloc`
- fixtures/reject/src/lib.rs:781:15: unsupported call to `<std::slice::IterMut<'_, u8> as std::iter::Iterator>::is_sorted`
- fixtures/reject/src/lib.rs:901:10: unsupported call to `<std::vec::Vec<u8> as std::clone::Clone>::clone_from`
- fixtures/reject/src/lib.rs:999:11: unsupported call to `std::ops::RangeTo::<u8>::contains::<Key>`
- fixtures/reject/src/lib.rs:999:32: unsupported call to `<std::ops::RangeFrom<u8> as std::ops::RangeBounds<u8>>::contains::<Key>`
- fixtures/reject/src/lib.rs:1004:9: unsupported call to `std::ops::Range::<&mut u8>::is_empty`
- fixtures/reject/src/lib.rs:1292:4: unsupported call to `core::num::<impl u8>::is_ascii_octdigit`

</details>

<details><summary>a raw pointer to `_` in a variable assigned again while it may point there (17)</summary>

- fixtures/reject/src/lib.rs:631:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:639:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:648:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:656:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:665:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:673:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:711:15: a raw pointer to `std::cell::UnsafeCell<u32>` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:724:15: a raw pointer to `std::cell::UnsafeCell<u32>` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1016:26: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1029:22: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1039:12: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1048:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1058:17: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1063:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1775:10: a raw pointer to `Target` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1788:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1819:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there

</details>

<details><summary>a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there (12)</summary>

- fixtures/reject/src/lib.rs:1352:33: a reference to `InArray`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1361:33: a reference to `InTuple`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1370:34: a reference to `InNested`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1379:11: a reference to `InPayload`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1391:33: a reference to `InField`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1672:19: a reference to `Owned`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1688:13: a reference to `Stored`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1704:10: a reference to `Passed`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1721:19: a reference to `Counter`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1746:18: a reference to `Core`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1761:18: a reference to `std::cell::UnsafeCell<u64>`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1804:25: a reference to `Picked`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there

</details>

<details><summary>a variant of `_`, which is not supported: either is the same JS string (4)</summary>

- fixtures/reject/src/lib.rs:344:2: a variant of `std::borrow::Cow<'_, str>`, which is not supported: either is the same JS string
- fixtures/reject/src/lib.rs:345:30: a variant of `std::borrow::Cow<'_, str>`, which is not supported: either is the same JS string
- fixtures/reject/src/lib.rs:346:27: a variant of `std::borrow::Cow<'_, str>`, which is not supported: either is the same JS string
- fixtures/reject/src/lib.rs:351:2: a variant of `std::borrow::Cow<'_, str>`, which is not supported: either is the same JS string

</details>

<details><summary>`_` with a mutable reference to a value JS copies, which its mapping would copy (3)</summary>

- fixtures/reject/src/lib.rs:529:12: `std::sync::Mutex::<T>::get_mut` with a mutable reference to a value JS copies, which its mapping would copy
- fixtures/reject/src/lib.rs:1250:22: `std::iter::once` with a mutable reference to a value JS copies, which its mapping would copy
- fixtures/reject/src/lib.rs:1658:3: `std::ops::IndexMut::index_mut` with a mutable reference to a value JS copies, which its mapping would copy

</details>

<details><summary>a copy of a closure that changes what it captured (3)</summary>

- fixtures/reject/src/lib.rs:237:14: a copy of a closure that changes what it captured
- fixtures/reject/src/lib.rs:522:14: a copy of a closure that changes what it captured
- fixtures/reject/src/lib.rs:523:14: a copy of a closure that changes what it captured

</details>

<details><summary>an assignment through a reference to `_` (3)</summary>

- fixtures/reject/src/lib.rs:90:2: an assignment through a reference to `std::sync::MutexGuard<'_, u8>`
- fixtures/reject/src/lib.rs:114:2: an assignment through a reference to `std::pin::Pin<std::boxed::Box<dyn std::future::Future<Output = u8>>>`
- fixtures/reject/src/lib.rs:221:2: an assignment through a reference to `std::pin::Pin<&mut Noisy>`

</details>

<details><summary>replacing the value behind a reference to `_` (3)</summary>

- fixtures/reject/src/lib.rs:98:2: replacing the value behind a reference to `std::pin::Pin<std::boxed::Box<std::future::PollFn<for<'a, 'b> fn(&'a mut std::task::Context<'b>) -> std::task::Poll<u8>>>>`
- fixtures/reject/src/lib.rs:106:2: replacing the value behind a reference to `std::pin::Pin<std::boxed::Box<[u8; 2]>>`
- fixtures/reject/src/lib.rs:119:2: replacing the value behind a reference to `std::sync::MutexGuard<'_, u8>`

</details>

<details><summary>unsupported type `_` (3)</summary>

- fixtures/reject/src/lib.rs:7:14: unsupported type `f16`
- fixtures/reject/src/lib.rs:7:25: unsupported type `f16`
- fixtures/reject/src/lib.rs:73:51: unsupported type `f16`

</details>

<details><summary>`Extend` of `_` by `_`, whose `_` of `_` is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:1282:31: `Extend` of `(std::vec::Vec<u8>, Tally)` by `(u8, u8)`, whose `Tally` of `u8` is not supported yet
- fixtures/reject/src/lib.rs:1287:29: `Extend` of `(Tally,)` by `(u8,)`, whose `Tally` of `u8` is not supported yet

</details>

<details><summary>`_` is not supported: its `None` and `Some` would both be `rt.None` (2)</summary>

- fixtures/reject/src/lib.rs:16:15: `Option<std::option::Option<std::option::Option<u8>>>` is not supported: its `None` and `Some` would both be `rt.None`
- fixtures/reject/src/lib.rs:17:2: `Option<std::option::Option<std::option::Option<u8>>>` is not supported: its `None` and `Some` would both be `rt.None`

</details>

<details><summary>`_` of `_`, whose `None` is `rt.None`, which is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:37:4: `Iterator::scan` of `Option<std::option::Option<u8>>`, whose `None` is `rt.None`, which is not supported yet
- fixtures/reject/src/lib.rs:243:11: `Iterator::scan` of `Option<()>`, whose `None` is `rt.None`, which is not supported yet

</details>

<details><summary>`_` of `_`, whose `clone_from` is its own, which is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:21:4: `slice::fill` of `Reused`, whose `clone_from` is its own, which is not supported yet
- fixtures/reject/src/lib.rs:299:4: `slice::clone_from_slice` of `Reused`, whose `clone_from` is its own, which is not supported yet

</details>

<details><summary>`_`, a trait object of another crate's trait, which is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:11:16: `dyn std::iter::Iterator<Item = u8>`, a trait object of another crate's trait, which is not supported yet
- fixtures/reject/src/lib.rs:12:4: `dyn std::iter::Iterator<Item = u8>`, a trait object of another crate's trait, which is not supported yet

</details>

<details><summary>`_`, a trait object of the generic trait `_`, which is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:472:23: `dyn Scaled<u8>`, a trait object of the generic trait `Scaled`, which is not supported yet
- fixtures/reject/src/lib.rs:473:4: `dyn Scaled<u8>`, a trait object of the generic trait `Scaled`, which is not supported yet

</details>

<details><summary>a coercion to `_`, which is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:463:40: a coercion to `std::sync::Weak<dyn Reading>`, which is not supported yet
- fixtures/reject/src/lib.rs:499:30: a coercion to `&Tail<dyn Reading>`, which is not supported yet

</details>

<details><summary>a raw pointer to a place of `()`, which has no address of its own (2)</summary>

- fixtures/reject/src/lib.rs:743:15: a raw pointer to a place of `()`, which has no address of its own
- fixtures/reject/src/lib.rs:743:29: a raw pointer to a place of `()`, which has no address of its own

</details>

<details><summary>an iterator of the crate implementing `_`, which is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:152:1: an iterator of the crate implementing `DoubleEndedIterator`, which is not supported yet
- fixtures/reject/src/lib.rs:200:1: an iterator of the crate implementing `FusedIterator`, which is not supported yet

</details>

<details><summary>overriding `_`, which std's adaptors call in place of `next` or beside it (2)</summary>

- fixtures/reject/src/lib.rs:132:2: overriding `Iterator::fold`, which std's adaptors call in place of `next` or beside it
- fixtures/reject/src/lib.rs:180:2: overriding `Iterator::size_hint`, which std's adaptors call in place of `next` or beside it

</details>

<details><summary>passing `_`, which drops, to `_`, which is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:339:4: passing `{closure@fixtures/reject/src/lib.rs:339:13: 339:27}`, which drops, to `str::contains`, which is not supported yet
- fixtures/reject/src/lib.rs:799:4: passing `{closure@fixtures/reject/src/lib.rs:799:11: 799:19}`, which drops, to `String::retain`, which is not supported yet

</details>

<details><summary>unsupported type `_` from another crate (2)</summary>

- fixtures/reject/src/lib.rs:446:32: unsupported type `std::net::AddrParseError` from another crate
- fixtures/reject/src/lib.rs:446:32: unsupported type `std::net::Ipv4Addr` from another crate

</details>

<details><summary>`Buf::copy_to_bytes` of `_`, a value JS copies, which is not supported (1)</summary>

- fixtures/reject/src/lib.rs:776:2: `Buf::copy_to_bytes` of `Blank`, a value JS copies, which is not supported

</details>

<details><summary>`BufMut::put` of `_` into a `BytesMut`, which picks the methods it calls by a capacity that depends on whether the other handles of its storage dropped, which is not supported (1)</summary>

- fixtures/reject/src/lib.rs:402:2: `BufMut::put` of `Counted` into a `BytesMut`, which picks the methods it calls by a capacity that depends on whether the other handles of its storage dropped, which is not supported

</details>

<details><summary>`Bytes::from_owner` of `_`, which would drop with the last clone of what it makes, which is not supported (1)</summary>

- fixtures/reject/src/lib.rs:387:2: `Bytes::from_owner` of `std::sync::Arc<[u8]>`, which would drop with the last clone of what it makes, which is not supported

</details>

<details><summary>`Pointer` formatting, as in `{:p}`, is not supported (1)</summary>

- fixtures/reject/src/lib.rs:56:2: `Pointer` formatting, as in `{:p}`, is not supported

</details>

<details><summary>`_` is not supported: its `Pending` and `Ready` would both be `rt.OuterPending` (1)</summary>

- fixtures/reject/src/lib.rs:79:20: `Poll<std::option::Option<std::task::Poll<std::task::Poll<u8>>>>` is not supported: its `Pending` and `Ready` would both be `rt.OuterPending`

</details>

<details><summary>`_` is not supported: its field `_` is `_`, typed `_`, whose `None` would be `rt.None` as `_` (1)</summary>

- fixtures/reject/src/lib.rs:31:2: `Maybe<()>` is not supported: its field `value` is `Option<T>`, typed `T | undefined`, whose `None` would be `rt.None` as `Option<()>`

</details>

<details><summary>`_` is not supported: its field `_` is `_`, typed `_`, whose `Pending` would be `rt.OuterPending` as `_` (1)</summary>

- fixtures/reject/src/lib.rs:510:2: `Waiting<std::task::Poll<u8>>` is not supported: its field `poll` is `Poll<T>`, typed `rt.Poll<T>`, whose `Pending` would be `rt.OuterPending` as `Poll<std::task::Poll<u8>>`

</details>

<details><summary>`_` of `_` by `_`, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:266:6: `HashMap::contains_key` of `std::collections::HashMap<Id, u8>` by `u8`, which is not supported yet

</details>

<details><summary>`_` of `_`, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:553:4: `std::any::Any::type_id` of `dyn std::any::Any`, which is not supported yet

</details>

<details><summary>`_` of a reference to an array of bytes it is generic over, which its mapping would take as the array, not as a slice of it (1)</summary>

- fixtures/reject/src/lib.rs:227:12: `std::option::Option::<&T>::copied` of a reference to an array of bytes it is generic over, which its mapping would take as the array, not as a slice of it

</details>

<details><summary>`_` of a value JS copies, which is no path (1)</summary>

- fixtures/reject/src/lib.rs:1652:2: `std::ops::DerefMut::deref_mut` of a value JS copies, which is no path

</details>

<details><summary>`_`, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:274:9: `Hasher::write`, which is not supported yet

</details>

<details><summary>`_`, which reads a capacity that depends on whether the other handles of its storage dropped, which is not supported (1)</summary>

- fixtures/reject/src/lib.rs:396:6: `BytesMut::capacity`, which reads a capacity that depends on whether the other handles of its storage dropped, which is not supported

</details>

<details><summary>`collect` of `_`, which sorts by `_`'s own `Ord`, of entries that drop, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:970:22: `collect` of `std::collections::BTreeMap<std::cell::Cell<Ranked>, Noisy>`, which sorts by `std::cell::Cell<Ranked>`'s own `Ord`, of entries that drop, which is not supported yet

</details>

<details><summary>`fmt::Arguments` with a placeholder, or not made in place, written to `_`'s own `fmt::Write`, which is not supported yet: std splits what a placeholder writes into `write_str` calls the runtime does not make (1)</summary>

- fixtures/reject/src/lib.rs:62:10: `fmt::Arguments` with a placeholder, or not made in place, written to `Sink`'s own `fmt::Write`, which is not supported yet: std splits what a placeholder writes into `write_str` calls the runtime does not make

</details>

<details><summary>a `collect` in place of `_`, a zip std reads by index, holding values that drop, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:216:40: a `collect` in place of `std::iter::Map<std::iter::Zip<std::iter::Map<std::vec::IntoIter<u8>, {closure@fixtures/reject/src/lib.rs:215:32: 215:40}>, std::ops::Range<u8>>, {closure@fixtures/reject/src/lib.rs:216:24: 216:32}>`, a zip std reads by index, holding values that drop, which is not supported yet

</details>

<details><summary>a `skip` of `_`, a zip std reads by index, of items that drop, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:209:8: a `skip` of `std::iter::Zip<std::iter::Map<std::slice::Iter<'_, u8>, {closure@fixtures/reject/src/lib.rs:208:27: 208:31}>, std::iter::Map<std::slice::Iter<'_, u8>, {closure@fixtures/reject/src/lib.rs:208:59: 208:63}>>`, a zip std reads by index, of items that drop, which is not supported yet

</details>

<details><summary>a cast from `_` to `_` (1)</summary>

- fixtures/reject/src/lib.rs:75:2: a cast from `u64` to `f16`

</details>

<details><summary>a cast from a pointer to `_` to one to `_`, which reads the bytes of one type as another (1)</summary>

- fixtures/reject/src/lib.rs:570:27: a cast from a pointer to `u32` to one to `u8`, which reads the bytes of one type as another

</details>

<details><summary>a coercion to `_` (1)</summary>

- fixtures/reject/src/lib.rs:304:2: a coercion to `std::sync::Arc<[u8]>`

</details>

<details><summary>a constant of type `_`, a pointer into memory translation can't name (1)</summary>

- fixtures/reject/src/lib.rs:793:3: a constant of type `&u32`, a pointer into memory translation can't name

</details>

<details><summary>a dangling pointer to `_`, a value of no bytes made of more than structs and tuples (1)</summary>

- fixtures/reject/src/lib.rs:752:11: a dangling pointer to `Only`, a value of no bytes made of more than structs and tuples

</details>

<details><summary>a future of `_`, which can be `rt.Pending`, as the future's own `Pending` is (1)</summary>

- fixtures/reject/src/lib.rs:84:34: a future of `std::task::Poll<u8>`, which can be `rt.Pending`, as the future's own `Pending` is

</details>

<details><summary>a mutable reference into a `Result`'s payload, passed on or kept (1)</summary>

- fixtures/reject/src/lib.rs:375:12: a mutable reference into a `Result`'s payload, passed on or kept

</details>

<details><summary>a mutable static, which any code may write (1)</summary>

- fixtures/reject/src/lib.rs:68:11: a mutable static, which any code may write

</details>

<details><summary>a pointer's address, which JS has no counterpart of (1)</summary>

- fixtures/reject/src/lib.rs:575:17: a pointer's address, which JS has no counterpart of

</details>

<details><summary>a printable type must implement `Display` or `Debug` (1)</summary>

- fixtures/reject/src/lib.rs:71:1: a printable type must implement `Display` or `Debug`

</details>

<details><summary>a raw pointer to `_` behind a reference, which is the object and not where it is kept (1)</summary>

- fixtures/reject/src/lib.rs:592:10: a raw pointer to `std::vec::Vec<u8>` behind a reference, which is the object and not where it is kept

</details>

<details><summary>a raw pointer to `_` in an element, a payload, or a union field, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:694:10: a raw pointer to `(u32, u32)` in an element, a payload, or a union field, which is not supported yet

</details>

<details><summary>a scalar constant of type `_` (1)</summary>

- fixtures/reject/src/lib.rs:8:2: a scalar constant of type `f16`

</details>

<details><summary>a trait object of `_`, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:489:26: a trait object of `[u8; 4]`, which is not supported yet

</details>

<details><summary>a zero-sized constant of type `_` (1)</summary>

- fixtures/reject/src/lib.rs:50:2: a zero-sized constant of type `{async closure@fixtures/reject/src/lib.rs:49:10: 49:18}`

</details>

<details><summary>an upcast from `_` to `_`, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:547:32: an upcast from `dyn Keyed` to `dyn std::any::Any`, which is not supported yet

</details>

<details><summary>arithmetic on `_` (1)</summary>

- fixtures/reject/src/lib.rs:8:2: arithmetic on `f16`

</details>

<details><summary>comparing `_` as a key, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:248:75: comparing `std::collections::BTreeSet<u8>` as a key, which is not supported yet

</details>

<details><summary>comparing pointers to `_` in an atomic, which JS has no address of (1)</summary>

- fixtures/reject/src/lib.rs:792:15: comparing pointers to `u32` in an atomic, which JS has no address of

</details>

<details><summary>comparing pointers to `_`, each a place JS makes of its own (1)</summary>

- fixtures/reject/src/lib.rs:611:2: comparing pointers to `u32`, each a place JS makes of its own

</details>

<details><summary>comparing pointers to `_`, which has no bytes and so no address of its own (1)</summary>

- fixtures/reject/src/lib.rs:743:70: comparing pointers to `Fused`, which has no bytes and so no address of its own

</details>

<details><summary>constructing `_` (1)</summary>

- fixtures/reject/src/lib.rs:362:53: constructing `std::borrow::Cow<'_, std::sync::Arc<u8>>`

</details>

<details><summary>hashing `_` as a key, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:248:75: hashing `std::collections::BTreeSet<u8>` as a key, which is not supported yet

</details>

<details><summary>hashing `_` through `_`'s own `Hash::hash_slice`, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:323:16: hashing `std::vec::Vec<Batched>` through `Batched`'s own `Hash::hash_slice`, which is not supported yet

</details>

<details><summary>passing `_`, a closure that owns a value that drops, to `_`, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:332:13: passing `{closure@fixtures/reject/src/lib.rs:332:28: 332:35}`, a closure that owns a value that drops, to `Entry::or_insert_with`, which is not supported yet

</details>

<details><summary>union `_`, whose fields overlap in `_` and `f64`, which JS keeps apart (1)</summary>

- fixtures/reject/src/lib.rs:597:1: union `Bits`, whose fields overlap in `u32` and `f64`, which JS keeps apart

</details>
