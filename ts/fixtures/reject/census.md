# Census of `reject-fixture`

- Refusals: 221
- Function instances: 282
- Generic functions no instance reached, lowered over their type parameters: 13

## By construct

| Count | Construct | First |
| ---: | --- | --- |
| 49 | a raw pointer to `_` in `_`, which the crate may put another object in place of | fixtures/reject/src/lib.rs:603:10 |
| 25 | unsupported call to `_` | fixtures/reject/src/lib.rs:160:8 |
| 19 | a raw pointer to `_` in a variable assigned again while it may point there | fixtures/reject/src/lib.rs:611:10 |
| 15 | a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of | fixtures/reject/src/lib.rs:1332:33 |
| 15 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | fixtures/reject/src/lib.rs:1650:18 |
| 5 | `_` with a mutable reference to a value JS copies, which its mapping would copy | fixtures/reject/src/lib.rs:1230:22 |
| 5 | a raw pointer to `_` in an element, a payload, or a union field, which is not supported yet | fixtures/reject/src/lib.rs:674:10 |
| 4 | a variant of `_`, which is not supported: either is the same JS string | fixtures/reject/src/lib.rs:331:2 |
| 3 | a copy of a closure that changes what it captured | fixtures/reject/src/lib.rs:224:14 |
| 3 | an assignment through a reference to `_` | fixtures/reject/src/lib.rs:83:2 |
| 3 | replacing the value behind a reference to `_` | fixtures/reject/src/lib.rs:91:2 |
| 3 | unsupported type `_` | fixtures/reject/src/lib.rs:7:14 |
| 2 | `_` is not supported: its `None` and `Some` would both be `rt.None` | fixtures/reject/src/lib.rs:16:15 |
| 2 | `_` of `_`, whose `None` is `rt.None`, which is not supported yet | fixtures/reject/src/lib.rs:37:4 |
| 2 | `_` of `_`, whose `clone_from` is its own, which is not supported yet | fixtures/reject/src/lib.rs:21:4 |
| 2 | `_`, a trait object of another crate's trait, which is not supported yet | fixtures/reject/src/lib.rs:11:16 |
| 2 | `_`, a trait object of the generic trait `_`, which is not supported yet | fixtures/reject/src/lib.rs:459:23 |
| 2 | a cast from a pointer to `_` to one to `_`, which reads the bytes of one type as another | fixtures/reject/src/lib.rs:550:27 |
| 2 | a coercion to `_`, which is not supported yet | fixtures/reject/src/lib.rs:450:40 |
| 2 | a raw pointer to a place of `()`, which has no address of its own | fixtures/reject/src/lib.rs:723:15 |
| 2 | an iterator of the crate implementing `_`, which is not supported yet | fixtures/reject/src/lib.rs:145:1 |
| 2 | comparing pointers to `_`, each a place JS makes of its own | fixtures/reject/src/lib.rs:591:2 |
| 2 | overriding `_`, which std's adaptors call in place of `next` or beside it | fixtures/reject/src/lib.rs:125:2 |
| 2 | passing `_`, which drops, to `_`, which is not supported yet | fixtures/reject/src/lib.rs:326:4 |
| 2 | unsupported type `_` from another crate | fixtures/reject/src/lib.rs:433:32 |
| 1 | `Buf::copy_to_bytes` of `_`, a value JS copies, which is not supported | fixtures/reject/src/lib.rs:756:2 |
| 1 | `BufMut::put` of `_` into a `BytesMut`, which picks the methods it calls by a capacity that depends on whether the other handles of its storage dropped, which is not supported | fixtures/reject/src/lib.rs:389:2 |
| 1 | `Bytes::from_owner` of `_`, which would drop with the last clone of what it makes, which is not supported | fixtures/reject/src/lib.rs:374:2 |
| 1 | `Extend` of `_` by `_`, whose `_` of `_` is not supported yet | fixtures/reject/src/lib.rs:1267:31 |
| 1 | `Pointer` formatting, as in `{:p}`, is not supported | fixtures/reject/src/lib.rs:49:2 |
| 1 | `_` is not supported: its `Pending` and `Ready` would both be `rt.OuterPending` | fixtures/reject/src/lib.rs:72:20 |
| 1 | `_` is not supported: its field `_` is `_`, typed `_`, whose `None` would be `rt.None` as `_` | fixtures/reject/src/lib.rs:31:2 |
| 1 | `_` is not supported: its field `_` is `_`, typed `_`, whose `Pending` would be `rt.OuterPending` as `_` | fixtures/reject/src/lib.rs:497:2 |
| 1 | `_` of `_` by `_`, which is not supported yet | fixtures/reject/src/lib.rs:253:6 |
| 1 | `_` of `_`, which is not supported yet | fixtures/reject/src/lib.rs:533:4 |
| 1 | `_`, which is not supported yet | fixtures/reject/src/lib.rs:261:9 |
| 1 | `_`, which reads a capacity that depends on whether the other handles of its storage dropped, which is not supported | fixtures/reject/src/lib.rs:383:6 |
| 1 | `clone_from` of `_`, whose items' `clone_from` is their own, which is not supported yet | fixtures/reject/src/lib.rs:2113:4 |
| 1 | `collect` of `_`, which sorts by `_`'s own `Ord`, of entries that drop, which is not supported yet | fixtures/reject/src/lib.rs:950:22 |
| 1 | `fmt::Arguments` with a placeholder, or not made in place, written to `_`'s own `fmt::Write`, which is not supported yet: std splits what a placeholder writes into `write_str` calls the runtime does not make | fixtures/reject/src/lib.rs:55:10 |
| 1 | a `collect` in place of `_`, a zip std reads by index, holding values that drop, which is not supported yet | fixtures/reject/src/lib.rs:209:40 |
| 1 | a `skip` of `_`, a zip std reads by index, of items that drop, which is not supported yet | fixtures/reject/src/lib.rs:202:8 |
| 1 | a cast from `_` to `_` | fixtures/reject/src/lib.rs:68:2 |
| 1 | a cast from `_` to `_`, which reads the bytes of one type as another | fixtures/reject/src/lib.rs:1708:33 |
| 1 | a coercion of `_` to `_`, a pointer to a slice | fixtures/reject/src/lib.rs:1709:28 |
| 1 | a coercion to `_` | fixtures/reject/src/lib.rs:291:2 |
| 1 | a constant of type `_`, a pointer into memory translation can't name | fixtures/reject/src/lib.rs:773:3 |
| 1 | a dangling pointer to `_`, a value of no bytes made of more than structs and tuples | fixtures/reject/src/lib.rs:732:11 |
| 1 | a future of `_`, which can be `rt.Pending`, as the future's own `Pending` is | fixtures/reject/src/lib.rs:77:34 |
| 1 | a mapped guard of `_`, which JS can't write in place | fixtures/reject/src/lib.rs:2147:10 |
| 1 | a mutable reference into a `Result`'s payload, passed on or kept | fixtures/reject/src/lib.rs:362:12 |
| 1 | a mutable static, which any code may write | fixtures/reject/src/lib.rs:61:11 |
| 1 | a pointer's address, which JS has no counterpart of | fixtures/reject/src/lib.rs:555:17 |
| 1 | a printable type must implement `Display` or `Debug` | fixtures/reject/src/lib.rs:64:1 |
| 1 | a raw pointer to `_` behind a reference, which is the object and not where it is kept | fixtures/reject/src/lib.rs:572:10 |
| 1 | a reference through which a raw pointer may be made into `_` in an element, a payload, or a union field, which is not supported yet | fixtures/reject/src/lib.rs:1359:11 |
| 1 | a scalar constant of type `_` | fixtures/reject/src/lib.rs:8:2 |
| 1 | a trait object of `_`, which is not supported yet | fixtures/reject/src/lib.rs:476:26 |
| 1 | an upcast from `_` to `_`, which is not supported yet | fixtures/reject/src/lib.rs:527:32 |
| 1 | arithmetic on `_` | fixtures/reject/src/lib.rs:8:2 |
| 1 | comparing `_` | fixtures/reject/src/lib.rs:1707:41 |
| 1 | comparing `_` as a key, which is not supported yet | fixtures/reject/src/lib.rs:235:75 |
| 1 | comparing pointers to `_` in an atomic, which JS has no address of | fixtures/reject/src/lib.rs:772:15 |
| 1 | comparing pointers to `_`, which has no bytes and so no address of its own | fixtures/reject/src/lib.rs:723:70 |
| 1 | const generic types are not supported: `_` | fixtures/reject/src/lib.rs:2092:1 |
| 1 | constructing `_` | fixtures/reject/src/lib.rs:349:53 |
| 1 | hashing `_` as a key, which is not supported yet | fixtures/reject/src/lib.rs:235:75 |
| 1 | hashing `_` by its address, which JS has no counterpart of | fixtures/reject/src/lib.rs:1711:5 |
| 1 | hashing `_` through `_`'s own `Hash::hash_slice`, which is not supported yet | fixtures/reject/src/lib.rs:310:16 |
| 1 | passing `_`, a closure that owns a value that drops, to `_`, which is not supported yet | fixtures/reject/src/lib.rs:319:13 |
| 1 | union `_`, whose fields overlap in `_` and `f64`, which JS keeps apart | fixtures/reject/src/lib.rs:577:1 |

## By API

The type or module each refused type or path belongs to.

| Count | API | First |
| ---: | --- | --- |
| 39 | `std::cell::UnsafeCell` | fixtures/reject/src/lib.rs:682:15 |
| 24 | `(u32, u32)` | fixtures/reject/src/lib.rs:603:10 |
| 6 | `[u8; 4]` | fixtures/reject/src/lib.rs:476:26 |
| 6 | `f16` | fixtures/reject/src/lib.rs:7:14 |
| 6 | `std::borrow::Cow` | fixtures/reject/src/lib.rs:331:2 |
| 6 | `u32` | fixtures/reject/src/lib.rs:550:27 |
| 5 | `(u8, u8)` | fixtures/reject/src/lib.rs:1267:31 |
| 5 | `Gap` | fixtures/reject/src/lib.rs:903:10 |
| 5 | `Option` | fixtures/reject/src/lib.rs:16:15 |
| 5 | `u8` | fixtures/reject/src/lib.rs:253:6 |
| 4 | `Iterator` | fixtures/reject/src/lib.rs:37:4 |
| 4 | `std::pin::Pin` | fixtures/reject/src/lib.rs:91:2 |
| 4 | `std::str` | fixtures/reject/src/lib.rs:434:4 |
| 4 | `{closure}` | fixtures/reject/src/lib.rs:319:13 |
| 3 | `*const [u8; 4]` | fixtures/reject/src/lib.rs:1708:33 |
| 3 | `Mid` | fixtures/reject/src/lib.rs:1350:34 |
| 3 | `Outer` | fixtures/reject/src/lib.rs:1350:34 |
| 3 | `std::ptr` | fixtures/reject/src/lib.rs:1705:3 |
| 3 | `std::vec::Vec` | fixtures/reject/src/lib.rs:310:16 |
| 2 | `(u16, u16)` | fixtures/reject/src/lib.rs:1171:31 |
| 2 | `InArray` | fixtures/reject/src/lib.rs:1297:9 |
| 2 | `InPayload` | fixtures/reject/src/lib.rs:1309:9 |
| 2 | `InTuple` | fixtures/reject/src/lib.rs:1301:9 |
| 2 | `InWritten` | fixtures/reject/src/lib.rs:1321:9 |
| 2 | `Inside` | fixtures/reject/src/lib.rs:1840:20 |
| 2 | `Poll` | fixtures/reject/src/lib.rs:72:20 |
| 2 | `Reused` | fixtures/reject/src/lib.rs:21:4 |
| 2 | `Scaled` | fixtures/reject/src/lib.rs:459:23 |
| 2 | `[u8; 2]` | fixtures/reject/src/lib.rs:1721:37 |
| 2 | `[u8; 3]` | fixtures/reject/src/lib.rs:1721:15 |
| 2 | `dyn Scaled` | fixtures/reject/src/lib.rs:459:23 |
| 2 | `dyn std::any::Any` | fixtures/reject/src/lib.rs:527:32 |
| 2 | `dyn std::iter::Iterator` | fixtures/reject/src/lib.rs:11:16 |
| 2 | `slice` | fixtures/reject/src/lib.rs:21:4 |
| 2 | `std::any::TypeId` | fixtures/reject/src/lib.rs:533:2 |
| 2 | `std::collections::BTreeSet` | fixtures/reject/src/lib.rs:235:75 |
| 2 | `std::collections::LinkedList` | fixtures/reject/src/lib.rs:2112:4 |
| 2 | `std::mem::MaybeUninit` | fixtures/reject/src/lib.rs:917:19 |
| 2 | `std::ops::Range` | fixtures/reject/src/lib.rs:160:8 |
| 2 | `std::option::Option` | fixtures/reject/src/lib.rs:369:2 |
| 2 | `std::ptr::const_ptr` | fixtures/reject/src/lib.rs:1708:73 |
| 2 | `std::std` | fixtures/reject/src/lib.rs:562:11 |
| 2 | `std::sync::Arc` | fixtures/reject/src/lib.rs:291:2 |
| 2 | `std::sync::MutexGuard` | fixtures/reject/src/lib.rs:83:2 |
| 2 | `std::vec::IntoIter` | fixtures/reject/src/lib.rs:429:17 |
| 1 | `(Dropped, (u8, u8))` | fixtures/reject/src/lib.rs:2193:2 |
| 1 | `([Cloned; 2], (u8, u8))` | fixtures/reject/src/lib.rs:2177:2 |
| 1 | `([Greatest; 2], (u8, u8))` | fixtures/reject/src/lib.rs:2211:13 |
| 1 | `([Touched; 2], (u8, u8))` | fixtures/reject/src/lib.rs:2242:2 |
| 1 | `(std::vec::Vec, Tens)` | fixtures/reject/src/lib.rs:1267:31 |
| 1 | `(u16, ByKeyReplace)` | fixtures/reject/src/lib.rs:1616:55 |
| 1 | `(u16, ByKeySet)` | fixtures/reject/src/lib.rs:1608:51 |
| 1 | `(u16, ByKeyTake)` | fixtures/reject/src/lib.rs:1623:52 |
| 1 | `(u16, ByRefReplace)` | fixtures/reject/src/lib.rs:1592:32 |
| 1 | `(u16, ByRefTake)` | fixtures/reject/src/lib.rs:1600:32 |
| 1 | `(u16, InTuple)` | fixtures/reject/src/lib.rs:1341:33 |
| 1 | `*const [u8]` | fixtures/reject/src/lib.rs:1709:28 |
| 1 | `*const u8` | fixtures/reject/src/lib.rs:1708:33 |
| 1 | `*mut [u8; 4]` | fixtures/reject/src/lib.rs:1707:41 |
| 1 | `Aliased` | fixtures/reject/src/lib.rs:1115:29 |
| 1 | `Assigned` | fixtures/reject/src/lib.rs:2007:20 |
| 1 | `Batched` | fixtures/reject/src/lib.rs:310:16 |
| 1 | `Behind` | fixtures/reject/src/lib.rs:1746:22 |
| 1 | `Bits` | fixtures/reject/src/lib.rs:577:1 |
| 1 | `Blank` | fixtures/reject/src/lib.rs:756:2 |
| 1 | `Block` | fixtures/reject/src/lib.rs:2092:1 |
| 1 | `ByBox` | fixtures/reject/src/lib.rs:1499:35 |
| 1 | `ByClosure` | fixtures/reject/src/lib.rs:1514:35 |
| 1 | `ByExchanged` | fixtures/reject/src/lib.rs:1560:30 |
| 1 | `ByInstance` | fixtures/reject/src/lib.rs:1540:35 |
| 1 | `ByKeyReplace` | fixtures/reject/src/lib.rs:1616:55 |
| 1 | `ByKeySet` | fixtures/reject/src/lib.rs:1608:51 |
| 1 | `ByKeyTake` | fixtures/reject/src/lib.rs:1623:52 |
| 1 | `ByObject` | fixtures/reject/src/lib.rs:1527:35 |
| 1 | `ByRead` | fixtures/reject/src/lib.rs:1450:35 |
| 1 | `ByRefReplace` | fixtures/reject/src/lib.rs:1592:32 |
| 1 | `ByRefTake` | fixtures/reject/src/lib.rs:1600:32 |
| 1 | `ByReplace` | fixtures/reject/src/lib.rs:1488:35 |
| 1 | `BySwap` | fixtures/reject/src/lib.rs:1476:35 |
| 1 | `BySwapped` | fixtures/reject/src/lib.rs:1549:76 |
| 1 | `ByWrite` | fixtures/reject/src/lib.rs:1463:35 |
| 1 | `BytesMut` | fixtures/reject/src/lib.rs:383:6 |
| 1 | `Called` | fixtures/reject/src/lib.rs:1081:28 |
| 1 | `Chosen` | fixtures/reject/src/lib.rs:1901:17 |
| 1 | `Closed` | fixtures/reject/src/lib.rs:2073:15 |
| 1 | `Compared` | fixtures/reject/src/lib.rs:1868:22 |
| 1 | `Copied` | fixtures/reject/src/lib.rs:1171:31 |
| 1 | `Counted` | fixtures/reject/src/lib.rs:389:2 |
| 1 | `Deep` | fixtures/reject/src/lib.rs:1782:18 |
| 1 | `Deeper` | fixtures/reject/src/lib.rs:2033:22 |
| 1 | `DoubleEndedIterator` | fixtures/reject/src/lib.rs:145:1 |
| 1 | `Dynamic` | fixtures/reject/src/lib.rs:2057:20 |
| 1 | `Enclosing` | fixtures/reject/src/lib.rs:1885:22 |
| 1 | `Entry` | fixtures/reject/src/lib.rs:319:13 |
| 1 | `Escaped` | fixtures/reject/src/lib.rs:1067:43 |
| 1 | `FusedIterator` | fixtures/reject/src/lib.rs:193:1 |
| 1 | `Fused` | fixtures/reject/src/lib.rs:723:70 |
| 1 | `HashMap` | fixtures/reject/src/lib.rs:253:6 |
| 1 | `Hasher` | fixtures/reject/src/lib.rs:261:9 |
| 1 | `Hits` | fixtures/reject/src/lib.rs:715:21 |
| 1 | `InField` | fixtures/reject/src/lib.rs:1313:9 |
| 1 | `InGeneric` | fixtures/reject/src/lib.rs:1317:9 |
| 1 | `InNested` | fixtures/reject/src/lib.rs:1305:9 |
| 1 | `Lane` | fixtures/reject/src/lib.rs:867:19 |
| 1 | `Loaded` | fixtures/reject/src/lib.rs:1137:29 |
| 1 | `Looped` | fixtures/reject/src/lib.rs:1105:26 |
| 1 | `Mapped` | fixtures/reject/src/lib.rs:1182:39 |
| 1 | `Maybe` | fixtures/reject/src/lib.rs:31:2 |
| 1 | `Onced` | fixtures/reject/src/lib.rs:1228:46 |
| 1 | `Only` | fixtures/reject/src/lib.rs:732:11 |
| 1 | `Parked` | fixtures/reject/src/lib.rs:2132:20 |
| 1 | `Parking` | fixtures/reject/src/lib.rs:2132:20 |
| 1 | `Plain` | fixtures/reject/src/lib.rs:821:10 |
| 1 | `Pushed` | fixtures/reject/src/lib.rs:1243:22 |
| 1 | `Repeated` | fixtures/reject/src/lib.rs:1216:18 |
| 1 | `Replaced` | fixtures/reject/src/lib.rs:1952:20 |
| 1 | `Rewritten` | fixtures/reject/src/lib.rs:1125:47 |
| 1 | `Shared` | fixtures/reject/src/lib.rs:1823:20 |
| 1 | `Sink` | fixtures/reject/src/lib.rs:55:10 |
| 1 | `Slotted` | fixtures/reject/src/lib.rs:1804:21 |
| 1 | `Span2` | fixtures/reject/src/lib.rs:603:10 |
| 1 | `String` | fixtures/reject/src/lib.rs:779:4 |
| 1 | `Swapped` | fixtures/reject/src/lib.rs:1970:20 |
| 1 | `T \| undefined` | fixtures/reject/src/lib.rs:31:2 |
| 1 | `Tail` | fixtures/reject/src/lib.rs:486:30 |
| 1 | `Taken` | fixtures/reject/src/lib.rs:1988:20 |
| 1 | `Target` | fixtures/reject/src/lib.rs:1664:10 |
| 1 | `Tens` | fixtures/reject/src/lib.rs:1267:31 |
| 1 | `Track` | fixtures/reject/src/lib.rs:847:19 |
| 1 | `Unboxed` | fixtures/reject/src/lib.rs:1761:16 |
| 1 | `Waiting` | fixtures/reject/src/lib.rs:497:2 |
| 1 | `Window` | fixtures/reject/src/lib.rs:664:2 |
| 1 | `Wrapped` | fixtures/reject/src/lib.rs:1929:21 |
| 1 | `[InArray; 2]` | fixtures/reject/src/lib.rs:1332:33 |
| 1 | `[InWritten; 1]` | fixtures/reject/src/lib.rs:1393:35 |
| 1 | `[std::cell::UnsafeCell; 2]` | fixtures/reject/src/lib.rs:2086:20 |
| 1 | `dyn Keyed` | fixtures/reject/src/lib.rs:527:32 |
| 1 | `poll` | fixtures/reject/src/lib.rs:497:2 |
| 1 | `rt.Poll` | fixtures/reject/src/lib.rs:497:2 |
| 1 | `std::any::Any` | fixtures/reject/src/lib.rs:533:4 |
| 1 | `std::cell::Cell` | fixtures/reject/src/lib.rs:950:22 |
| 1 | `std::collections::BTreeMap` | fixtures/reject/src/lib.rs:950:22 |
| 1 | `std::collections::HashMap` | fixtures/reject/src/lib.rs:253:6 |
| 1 | `std::default::Default` | fixtures/reject/src/lib.rs:2114:2 |
| 1 | `std::iter::Map` | fixtures/reject/src/lib.rs:209:40 |
| 1 | `std::iter::Zip` | fixtures/reject/src/lib.rs:202:8 |
| 1 | `std::iter` | fixtures/reject/src/lib.rs:1230:22 |
| 1 | `std::net::AddrParseError` | fixtures/reject/src/lib.rs:433:32 |
| 1 | `std::net::Ipv4Addr` | fixtures/reject/src/lib.rs:433:32 |
| 1 | `std::num` | fixtures/reject/src/lib.rs:1272:4 |
| 1 | `std::ops::RangeFrom` | fixtures/reject/src/lib.rs:979:32 |
| 1 | `std::ops::RangeTo` | fixtures/reject/src/lib.rs:979:11 |
| 1 | `std::ptr::mut_ptr` | fixtures/reject/src/lib.rs:1708:56 |
| 1 | `std::slice::IterMut` | fixtures/reject/src/lib.rs:761:15 |
| 1 | `std::std::Layout` | fixtures/reject/src/lib.rs:560:15 |
| 1 | `std::sync::Weak` | fixtures/reject/src/lib.rs:450:40 |
| 1 | `std::task::Poll` | fixtures/reject/src/lib.rs:77:34 |
| 1 | `str` | fixtures/reject/src/lib.rs:326:4 |
| 1 | `u64` | fixtures/reject/src/lib.rs:68:2 |
| 1 | `value` | fixtures/reject/src/lib.rs:31:2 |

## By type or path

| Count | Construct | Type or path | First |
| ---: | --- | --- | --- |
| 14 | a raw pointer to `_` in a variable assigned again while it may point there | `(u32, u32)` | fixtures/reject/src/lib.rs:611:10 |
| 4 | a variant of `_`, which is not supported: either is the same JS string | `std::borrow::Cow` | fixtures/reject/src/lib.rs:331:2 |
| 3 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u32, u32)`, `Gap` | fixtures/reject/src/lib.rs:903:10 |
| 3 | a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of | `Mid`, `Outer` | fixtures/reject/src/lib.rs:1350:34 |
| 3 | unsupported type `_` | `f16` | fixtures/reject/src/lib.rs:7:14 |
| 2 | `_` is not supported: its `None` and `Some` would both be `rt.None` | `Option` | fixtures/reject/src/lib.rs:16:15 |
| 2 | `_` of `_`, whose `None` is `rt.None`, which is not supported yet | `Iterator::scan`, `Option` | fixtures/reject/src/lib.rs:37:4 |
| 2 | `_`, a trait object of another crate's trait, which is not supported yet | `dyn std::iter::Iterator` | fixtures/reject/src/lib.rs:11:16 |
| 2 | `_`, a trait object of the generic trait `_`, which is not supported yet | `dyn Scaled`, `Scaled` | fixtures/reject/src/lib.rs:459:23 |
| 2 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::mem::MaybeUninit`, `Gap` | fixtures/reject/src/lib.rs:917:19 |
| 2 | a raw pointer to `_` in a variable assigned again while it may point there | `[u8; 4]` | fixtures/reject/src/lib.rs:1728:26 |
| 2 | a raw pointer to `_` in a variable assigned again while it may point there | `std::cell::UnsafeCell` | fixtures/reject/src/lib.rs:691:15 |
| 2 | a raw pointer to `_` in an element, a payload, or a union field, which is not supported yet | `[u8; 2]` | fixtures/reject/src/lib.rs:1721:37 |
| 2 | a raw pointer to `_` in an element, a payload, or a union field, which is not supported yet | `[u8; 3]` | fixtures/reject/src/lib.rs:1721:15 |
| 2 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Inside` | fixtures/reject/src/lib.rs:1840:20 |
| 2 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `[u8; 4]` | fixtures/reject/src/lib.rs:1728:26 |
| 2 | an assignment through a reference to `_` | `std::pin::Pin` | fixtures/reject/src/lib.rs:107:2 |
| 2 | comparing pointers to `_`, each a place JS makes of its own | `u32` | fixtures/reject/src/lib.rs:591:2 |
| 2 | replacing the value behind a reference to `_` | `std::pin::Pin` | fixtures/reject/src/lib.rs:91:2 |
| 2 | unsupported call to `_` | `<std::vec::IntoIter as std::iter::Iterator>::sum` | fixtures/reject/src/lib.rs:429:17 |
| 1 | `Buf::copy_to_bytes` of `_`, a value JS copies, which is not supported | `Blank` | fixtures/reject/src/lib.rs:756:2 |
| 1 | `BufMut::put` of `_` into a `BytesMut`, which picks the methods it calls by a capacity that depends on whether the other handles of its storage dropped, which is not supported | `Counted` | fixtures/reject/src/lib.rs:389:2 |
| 1 | `Bytes::from_owner` of `_`, which would drop with the last clone of what it makes, which is not supported | `std::sync::Arc` | fixtures/reject/src/lib.rs:374:2 |
| 1 | `Extend` of `_` by `_`, whose `_` of `_` is not supported yet | `(std::vec::Vec, Tens)`, `(u8, u8)`, `Tens`, `u8` | fixtures/reject/src/lib.rs:1267:31 |
| 1 | `_` is not supported: its `Pending` and `Ready` would both be `rt.OuterPending` | `Poll` | fixtures/reject/src/lib.rs:72:20 |
| 1 | `_` is not supported: its field `_` is `_`, typed `_`, whose `None` would be `rt.None` as `_` | `Maybe`, `value`, `Option`, `T \| undefined`, `Option` | fixtures/reject/src/lib.rs:31:2 |
| 1 | `_` is not supported: its field `_` is `_`, typed `_`, whose `Pending` would be `rt.OuterPending` as `_` | `Waiting`, `poll`, `Poll`, `rt.Poll`, `Poll` | fixtures/reject/src/lib.rs:497:2 |
| 1 | `_` of `_` by `_`, which is not supported yet | `HashMap::contains_key`, `std::collections::HashMap`, `u8` | fixtures/reject/src/lib.rs:253:6 |
| 1 | `_` of `_`, which is not supported yet | `std::any::Any::type_id`, `dyn std::any::Any` | fixtures/reject/src/lib.rs:533:4 |
| 1 | `_` of `_`, whose `clone_from` is its own, which is not supported yet | `slice::clone_from_slice`, `Reused` | fixtures/reject/src/lib.rs:286:4 |
| 1 | `_` of `_`, whose `clone_from` is its own, which is not supported yet | `slice::fill`, `Reused` | fixtures/reject/src/lib.rs:21:4 |
| 1 | `_` with a mutable reference to a value JS copies, which its mapping would copy | `std::collections::LinkedList::retain` | fixtures/reject/src/lib.rs:2112:4 |
| 1 | `_` with a mutable reference to a value JS copies, which its mapping would copy | `std::default::Default::default` | fixtures/reject/src/lib.rs:2114:2 |
| 1 | `_` with a mutable reference to a value JS copies, which its mapping would copy | `std::iter::once` | fixtures/reject/src/lib.rs:1230:22 |
| 1 | `_` with a mutable reference to a value JS copies, which its mapping would copy | `std::str::from_utf8_mut` | fixtures/reject/src/lib.rs:1637:20 |
| 1 | `_` with a mutable reference to a value JS copies, which its mapping would copy | `std::str::get_unchecked_mut` | fixtures/reject/src/lib.rs:1632:13 |
| 1 | `_`, which is not supported yet | `Hasher::write` | fixtures/reject/src/lib.rs:261:9 |
| 1 | `_`, which reads a capacity that depends on whether the other handles of its storage dropped, which is not supported | `BytesMut::capacity` | fixtures/reject/src/lib.rs:383:6 |
| 1 | `clone_from` of `_`, whose items' `clone_from` is their own, which is not supported yet | `std::collections::LinkedList` | fixtures/reject/src/lib.rs:2113:4 |
| 1 | `collect` of `_`, which sorts by `_`'s own `Ord`, of entries that drop, which is not supported yet | `std::collections::BTreeMap`, `std::cell::Cell` | fixtures/reject/src/lib.rs:950:22 |
| 1 | `fmt::Arguments` with a placeholder, or not made in place, written to `_`'s own `fmt::Write`, which is not supported yet: std splits what a placeholder writes into `write_str` calls the runtime does not make | `Sink` | fixtures/reject/src/lib.rs:55:10 |
| 1 | a `collect` in place of `_`, a zip std reads by index, holding values that drop, which is not supported yet | `std::iter::Map` | fixtures/reject/src/lib.rs:209:40 |
| 1 | a `skip` of `_`, a zip std reads by index, of items that drop, which is not supported yet | `std::iter::Zip` | fixtures/reject/src/lib.rs:202:8 |
| 1 | a cast from `_` to `_` | `u64`, `f16` | fixtures/reject/src/lib.rs:68:2 |
| 1 | a cast from `_` to `_`, which reads the bytes of one type as another | `*const [u8; 4]`, `*const u8` | fixtures/reject/src/lib.rs:1708:33 |
| 1 | a cast from a pointer to `_` to one to `_`, which reads the bytes of one type as another | `[u8; 4]`, `u8` | fixtures/reject/src/lib.rs:1708:17 |
| 1 | a cast from a pointer to `_` to one to `_`, which reads the bytes of one type as another | `u32`, `u8` | fixtures/reject/src/lib.rs:550:27 |
| 1 | a coercion of `_` to `_`, a pointer to a slice | `*const [u8; 4]`, `*const [u8]` | fixtures/reject/src/lib.rs:1709:28 |
| 1 | a coercion to `_` | `std::sync::Arc` | fixtures/reject/src/lib.rs:291:2 |
| 1 | a coercion to `_`, which is not supported yet | `&Tail` | fixtures/reject/src/lib.rs:486:30 |
| 1 | a coercion to `_`, which is not supported yet | `std::sync::Weak` | fixtures/reject/src/lib.rs:450:40 |
| 1 | a constant of type `_`, a pointer into memory translation can't name | `&u32` | fixtures/reject/src/lib.rs:773:3 |
| 1 | a dangling pointer to `_`, a value of no bytes made of more than structs and tuples | `Only` | fixtures/reject/src/lib.rs:732:11 |
| 1 | a future of `_`, which can be `rt.Pending`, as the future's own `Pending` is | `std::task::Poll` | fixtures/reject/src/lib.rs:77:34 |
| 1 | a mapped guard of `_`, which JS can't write in place | `{closure}` | fixtures/reject/src/lib.rs:2147:10 |
| 1 | a raw pointer to `_` behind a reference, which is the object and not where it is kept | `std::vec::Vec` | fixtures/reject/src/lib.rs:572:10 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u16, u16)`, `Copied` | fixtures/reject/src/lib.rs:1171:31 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u16, u16)`, `Pushed` | fixtures/reject/src/lib.rs:1243:22 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u32, u32)`, `Lane` | fixtures/reject/src/lib.rs:867:19 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u32, u32)`, `Plain` | fixtures/reject/src/lib.rs:821:10 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u32, u32)`, `Span2` | fixtures/reject/src/lib.rs:603:10 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u32, u32)`, `Track` | fixtures/reject/src/lib.rs:847:19 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u32, u32)`, `Window` | fixtures/reject/src/lib.rs:664:2 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u32, u32)`, `std::cell::UnsafeCell` | fixtures/reject/src/lib.rs:682:15 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u8, u8)`, `(Dropped, (u8, u8))` | fixtures/reject/src/lib.rs:2193:2 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u8, u8)`, `([Cloned; 2], (u8, u8))` | fixtures/reject/src/lib.rs:2177:2 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u8, u8)`, `([Greatest; 2], (u8, u8))` | fixtures/reject/src/lib.rs:2211:13 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `(u8, u8)`, `([Touched; 2], (u8, u8))` | fixtures/reject/src/lib.rs:2242:2 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `ByKeyReplace`, `(u16, ByKeyReplace)` | fixtures/reject/src/lib.rs:1616:55 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `ByKeySet`, `(u16, ByKeySet)` | fixtures/reject/src/lib.rs:1608:51 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `ByKeyTake`, `(u16, ByKeyTake)` | fixtures/reject/src/lib.rs:1623:52 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `ByRefReplace`, `(u16, ByRefReplace)` | fixtures/reject/src/lib.rs:1592:32 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `ByRefTake`, `(u16, ByRefTake)` | fixtures/reject/src/lib.rs:1600:32 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Aliased` | fixtures/reject/src/lib.rs:1115:29 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByBox` | fixtures/reject/src/lib.rs:1499:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByClosure` | fixtures/reject/src/lib.rs:1514:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByExchanged` | fixtures/reject/src/lib.rs:1560:30 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByInstance` | fixtures/reject/src/lib.rs:1540:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByObject` | fixtures/reject/src/lib.rs:1527:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByRead` | fixtures/reject/src/lib.rs:1450:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByReplace` | fixtures/reject/src/lib.rs:1488:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `BySwap` | fixtures/reject/src/lib.rs:1476:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `BySwapped` | fixtures/reject/src/lib.rs:1549:76 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `ByWrite` | fixtures/reject/src/lib.rs:1463:35 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Called` | fixtures/reject/src/lib.rs:1081:28 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Escaped` | fixtures/reject/src/lib.rs:1067:43 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Hits` | fixtures/reject/src/lib.rs:715:21 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `InArray` | fixtures/reject/src/lib.rs:1297:9 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `InField` | fixtures/reject/src/lib.rs:1313:9 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `InGeneric` | fixtures/reject/src/lib.rs:1317:9 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `InNested` | fixtures/reject/src/lib.rs:1305:9 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `InPayload` | fixtures/reject/src/lib.rs:1309:9 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `InTuple` | fixtures/reject/src/lib.rs:1301:9 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `InWritten` | fixtures/reject/src/lib.rs:1321:9 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Loaded` | fixtures/reject/src/lib.rs:1137:29 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Looped` | fixtures/reject/src/lib.rs:1105:26 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Mapped` | fixtures/reject/src/lib.rs:1182:39 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Onced` | fixtures/reject/src/lib.rs:1228:46 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Repeated` | fixtures/reject/src/lib.rs:1216:18 |
| 1 | a raw pointer to `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Rewritten` | fixtures/reject/src/lib.rs:1125:47 |
| 1 | a raw pointer to `_` in a variable assigned again while it may point there | `Target` | fixtures/reject/src/lib.rs:1664:10 |
| 1 | a raw pointer to `_` in an element, a payload, or a union field, which is not supported yet | `(u32, u32)` | fixtures/reject/src/lib.rs:674:10 |
| 1 | a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of | `InArray`, `[InArray; 2]` | fixtures/reject/src/lib.rs:1332:33 |
| 1 | a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of | `InTuple`, `(u16, InTuple)` | fixtures/reject/src/lib.rs:1341:33 |
| 1 | a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of | `InWritten`, `[InWritten; 1]` | fixtures/reject/src/lib.rs:1393:35 |
| 1 | a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of | `Parked`, `Parking` | fixtures/reject/src/lib.rs:2132:20 |
| 1 | a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Assigned` | fixtures/reject/src/lib.rs:2007:20 |
| 1 | a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Closed` | fixtures/reject/src/lib.rs:2073:15 |
| 1 | a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Deeper` | fixtures/reject/src/lib.rs:2033:22 |
| 1 | a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Dynamic` | fixtures/reject/src/lib.rs:2057:20 |
| 1 | a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Replaced` | fixtures/reject/src/lib.rs:1952:20 |
| 1 | a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Swapped` | fixtures/reject/src/lib.rs:1970:20 |
| 1 | a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `Taken` | fixtures/reject/src/lib.rs:1988:20 |
| 1 | a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of | `std::cell::UnsafeCell`, `[std::cell::UnsafeCell; 2]` | fixtures/reject/src/lib.rs:2086:20 |
| 1 | a reference through which a raw pointer may be made into `_` in an element, a payload, or a union field, which is not supported yet | `InPayload` | fixtures/reject/src/lib.rs:1359:11 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Behind` | fixtures/reject/src/lib.rs:1746:22 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Chosen` | fixtures/reject/src/lib.rs:1901:17 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Compared` | fixtures/reject/src/lib.rs:1868:22 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Deep` | fixtures/reject/src/lib.rs:1782:18 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Enclosing` | fixtures/reject/src/lib.rs:1885:22 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Shared` | fixtures/reject/src/lib.rs:1823:20 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Slotted` | fixtures/reject/src/lib.rs:1804:21 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Unboxed` | fixtures/reject/src/lib.rs:1761:16 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `Wrapped` | fixtures/reject/src/lib.rs:1929:21 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `std::cell::UnsafeCell` | fixtures/reject/src/lib.rs:1650:18 |
| 1 | a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there | `std::option::Option` | fixtures/reject/src/lib.rs:2103:13 |
| 1 | a scalar constant of type `_` | `f16` | fixtures/reject/src/lib.rs:8:2 |
| 1 | a trait object of `_`, which is not supported yet | `[u8; 4]` | fixtures/reject/src/lib.rs:476:26 |
| 1 | an assignment through a reference to `_` | `std::sync::MutexGuard` | fixtures/reject/src/lib.rs:83:2 |
| 1 | an iterator of the crate implementing `_`, which is not supported yet | `DoubleEndedIterator` | fixtures/reject/src/lib.rs:145:1 |
| 1 | an iterator of the crate implementing `_`, which is not supported yet | `FusedIterator` | fixtures/reject/src/lib.rs:193:1 |
| 1 | an upcast from `_` to `_`, which is not supported yet | `dyn Keyed`, `dyn std::any::Any` | fixtures/reject/src/lib.rs:527:32 |
| 1 | arithmetic on `_` | `f16` | fixtures/reject/src/lib.rs:8:2 |
| 1 | comparing `_` | `*mut [u8; 4]` | fixtures/reject/src/lib.rs:1707:41 |
| 1 | comparing `_` as a key, which is not supported yet | `std::collections::BTreeSet` | fixtures/reject/src/lib.rs:235:75 |
| 1 | comparing pointers to `_` in an atomic, which JS has no address of | `u32` | fixtures/reject/src/lib.rs:772:15 |
| 1 | comparing pointers to `_`, which has no bytes and so no address of its own | `Fused` | fixtures/reject/src/lib.rs:723:70 |
| 1 | const generic types are not supported: `_` | `Block` | fixtures/reject/src/lib.rs:2092:1 |
| 1 | constructing `_` | `std::borrow::Cow` | fixtures/reject/src/lib.rs:349:53 |
| 1 | hashing `_` as a key, which is not supported yet | `std::collections::BTreeSet` | fixtures/reject/src/lib.rs:235:75 |
| 1 | hashing `_` by its address, which JS has no counterpart of | `*const [u8; 4]` | fixtures/reject/src/lib.rs:1711:5 |
| 1 | hashing `_` through `_`'s own `Hash::hash_slice`, which is not supported yet | `std::vec::Vec`, `Batched` | fixtures/reject/src/lib.rs:310:16 |
| 1 | overriding `_`, which std's adaptors call in place of `next` or beside it | `Iterator::fold` | fixtures/reject/src/lib.rs:125:2 |
| 1 | overriding `_`, which std's adaptors call in place of `next` or beside it | `Iterator::size_hint` | fixtures/reject/src/lib.rs:173:2 |
| 1 | passing `_`, a closure that owns a value that drops, to `_`, which is not supported yet | `{closure}`, `Entry::or_insert_with` | fixtures/reject/src/lib.rs:319:13 |
| 1 | passing `_`, which drops, to `_`, which is not supported yet | `{closure}`, `String::retain` | fixtures/reject/src/lib.rs:779:4 |
| 1 | passing `_`, which drops, to `_`, which is not supported yet | `{closure}`, `str::contains` | fixtures/reject/src/lib.rs:326:4 |
| 1 | replacing the value behind a reference to `_` | `std::sync::MutexGuard` | fixtures/reject/src/lib.rs:112:2 |
| 1 | union `_`, whose fields overlap in `_` and `f64`, which JS keeps apart | `Bits`, `u32` | fixtures/reject/src/lib.rs:577:1 |
| 1 | unsupported call to `_` | `<std::any::TypeId as std::cmp::PartialEq>::eq` | fixtures/reject/src/lib.rs:533:2 |
| 1 | unsupported call to `_` | `<std::ops::RangeFrom as std::ops::RangeBounds>::contains` | fixtures/reject/src/lib.rs:979:32 |
| 1 | unsupported call to `_` | `<std::option::Option as std::cmp::PartialEq>::eq` | fixtures/reject/src/lib.rs:369:2 |
| 1 | unsupported call to `_` | `<std::slice::IterMut as std::iter::Iterator>::is_sorted` | fixtures/reject/src/lib.rs:761:15 |
| 1 | unsupported call to `_` | `<std::vec::Vec as std::clone::Clone>::clone_from` | fixtures/reject/src/lib.rs:881:10 |
| 1 | unsupported call to `_` | `<u8 as std::hash::Hash>::hash` | fixtures/reject/src/lib.rs:301:10 |
| 1 | unsupported call to `_` | `std::any::TypeId::of` | fixtures/reject/src/lib.rs:533:17 |
| 1 | unsupported call to `_` | `std::borrow::Cow::is_borrowed` | fixtures/reject/src/lib.rs:343:2 |
| 1 | unsupported call to `_` | `std::num::is_ascii_octdigit` | fixtures/reject/src/lib.rs:1272:4 |
| 1 | unsupported call to `_` | `std::ops::Range::contains` | fixtures/reject/src/lib.rs:160:8 |
| 1 | unsupported call to `_` | `std::ops::Range::is_empty` | fixtures/reject/src/lib.rs:984:9 |
| 1 | unsupported call to `_` | `std::ops::RangeTo::contains` | fixtures/reject/src/lib.rs:979:11 |
| 1 | unsupported call to `_` | `std::ptr::addr_eq` | fixtures/reject/src/lib.rs:1707:14 |
| 1 | unsupported call to `_` | `std::ptr::const_ptr::len` | fixtures/reject/src/lib.rs:1712:58 |
| 1 | unsupported call to `_` | `std::ptr::const_ptr::offset` | fixtures/reject/src/lib.rs:1708:73 |
| 1 | unsupported call to `_` | `std::ptr::copy_nonoverlapping` | fixtures/reject/src/lib.rs:1706:3 |
| 1 | unsupported call to `_` | `std::ptr::copy` | fixtures/reject/src/lib.rs:1705:3 |
| 1 | unsupported call to `_` | `std::ptr::mut_ptr::add` | fixtures/reject/src/lib.rs:1708:56 |
| 1 | unsupported call to `_` | `std::std::Layout::new` | fixtures/reject/src/lib.rs:560:15 |
| 1 | unsupported call to `_` | `std::std::alloc` | fixtures/reject/src/lib.rs:562:11 |
| 1 | unsupported call to `_` | `std::std::dealloc` | fixtures/reject/src/lib.rs:565:3 |
| 1 | unsupported call to `_` | `std::str::as_bytes_mut` | fixtures/reject/src/lib.rs:1631:13 |
| 1 | unsupported call to `_` | `std::str::parse` | fixtures/reject/src/lib.rs:434:4 |
| 1 | unsupported type `_` from another crate | `std::net::AddrParseError` | fixtures/reject/src/lib.rs:433:32 |
| 1 | unsupported type `_` from another crate | `std::net::Ipv4Addr` | fixtures/reject/src/lib.rs:433:32 |

## Spans

<details><summary>a raw pointer to `_` in `_`, which the crate may put another object in place of (49)</summary>

- fixtures/reject/src/lib.rs:603:10: a raw pointer to `(u32, u32)` in `Span2`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:664:2: a raw pointer to `(u32, u32)` in `Window`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:682:15: a raw pointer to `(u32, u32)` in `std::cell::UnsafeCell<(u32, u32)>`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:715:21: a raw pointer to `std::cell::UnsafeCell<u32>` in `Hits`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:821:10: a raw pointer to `(u32, u32)` in `Plain`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:847:19: a raw pointer to `(u32, u32)` in `Track`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:867:19: a raw pointer to `(u32, u32)` in `Lane`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:903:10: a raw pointer to `(u32, u32)` in `Gap`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:904:10: a raw pointer to `(u32, u32)` in `Gap`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:905:58: a raw pointer to `(u32, u32)` in `Gap`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:917:19: a raw pointer to `std::mem::MaybeUninit<(u32, u32)>` in `Gap`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:918:19: a raw pointer to `std::mem::MaybeUninit<(u32, u32)>` in `Gap`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1067:43: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Escaped`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1081:28: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Called`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1105:26: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Looped`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1115:29: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Aliased`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1125:47: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Rewritten`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1137:29: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Loaded`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1171:31: a raw pointer to `(u16, u16)` in `Copied`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1182:39: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Mapped`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1216:18: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Repeated`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1228:46: a raw pointer to `std::cell::UnsafeCell<(u16, u16)>` in `Onced`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1243:22: a raw pointer to `(u16, u16)` in `Pushed`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1297:9: a raw pointer to `std::cell::UnsafeCell<u16>` in `InArray`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1301:9: a raw pointer to `std::cell::UnsafeCell<u16>` in `InTuple`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1305:9: a raw pointer to `std::cell::UnsafeCell<u16>` in `InNested`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1309:9: a raw pointer to `std::cell::UnsafeCell<u16>` in `InPayload`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1313:9: a raw pointer to `std::cell::UnsafeCell<u16>` in `InField`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1317:9: a raw pointer to `std::cell::UnsafeCell<u16>` in `InGeneric`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1321:9: a raw pointer to `std::cell::UnsafeCell<u16>` in `InWritten`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1450:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByRead`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1463:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByWrite`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1476:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `BySwap`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1488:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByReplace`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1499:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByBox`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1514:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByClosure`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1527:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByObject`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1540:35: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByInstance`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1549:76: a raw pointer to `std::cell::UnsafeCell<u16>` in `BySwapped`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1560:30: a raw pointer to `std::cell::UnsafeCell<u16>` in `ByExchanged`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1592:32: a raw pointer to `ByRefReplace` in `(u16, ByRefReplace)`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1600:32: a raw pointer to `ByRefTake` in `(u16, ByRefTake)`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1608:51: a raw pointer to `ByKeySet` in `(u16, ByKeySet)`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1616:55: a raw pointer to `ByKeyReplace` in `(u16, ByKeyReplace)`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1623:52: a raw pointer to `ByKeyTake` in `(u16, ByKeyTake)`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:2177:2: a raw pointer to `(u8, u8)` in `([Cloned; 2], (u8, u8))`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:2193:2: a raw pointer to `(u8, u8)` in `(Dropped, (u8, u8))`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:2211:13: a raw pointer to `(u8, u8)` in `([Greatest; 2], (u8, u8))`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:2242:2: a raw pointer to `(u8, u8)` in `([Touched; 2], (u8, u8))`, which the crate may put another object in place of

</details>

<details><summary>unsupported call to `_` (25)</summary>

- fixtures/reject/src/lib.rs:160:8: unsupported call to `std::ops::Range::<Key>::contains::<Key>`
- fixtures/reject/src/lib.rs:301:10: unsupported call to `<u8 as std::hash::Hash>::hash::<H>`
- fixtures/reject/src/lib.rs:343:2: unsupported call to `std::borrow::Cow::<'_, [u8]>::is_borrowed`
- fixtures/reject/src/lib.rs:369:2: unsupported call to `<std::option::Option<&mut u32> as std::cmp::PartialEq>::eq`
- fixtures/reject/src/lib.rs:429:17: unsupported call to `<std::vec::IntoIter<Part> as std::iter::Iterator>::sum::<std::time::Duration>`
- fixtures/reject/src/lib.rs:429:38: unsupported call to `<std::vec::IntoIter<Part> as std::iter::Iterator>::sum::<u64>`
- fixtures/reject/src/lib.rs:434:4: unsupported call to `core::str::<impl str>::parse::<std::net::Ipv4Addr>`
- fixtures/reject/src/lib.rs:533:2: unsupported call to `<std::any::TypeId as std::cmp::PartialEq>::eq`
- fixtures/reject/src/lib.rs:533:17: unsupported call to `std::any::TypeId::of::<u8>`
- fixtures/reject/src/lib.rs:560:15: unsupported call to `std::alloc::Layout::new::<u8>`
- fixtures/reject/src/lib.rs:562:11: unsupported call to `std::alloc::alloc`
- fixtures/reject/src/lib.rs:565:3: unsupported call to `std::alloc::dealloc`
- fixtures/reject/src/lib.rs:761:15: unsupported call to `<std::slice::IterMut<'_, u8> as std::iter::Iterator>::is_sorted`
- fixtures/reject/src/lib.rs:881:10: unsupported call to `<std::vec::Vec<u8> as std::clone::Clone>::clone_from`
- fixtures/reject/src/lib.rs:979:11: unsupported call to `std::ops::RangeTo::<u8>::contains::<Key>`
- fixtures/reject/src/lib.rs:979:32: unsupported call to `<std::ops::RangeFrom<u8> as std::ops::RangeBounds<u8>>::contains::<Key>`
- fixtures/reject/src/lib.rs:984:9: unsupported call to `std::ops::Range::<&mut u8>::is_empty`
- fixtures/reject/src/lib.rs:1272:4: unsupported call to `core::num::<impl u8>::is_ascii_octdigit`
- fixtures/reject/src/lib.rs:1631:13: unsupported call to `core::str::<impl str>::as_bytes_mut`
- fixtures/reject/src/lib.rs:1705:3: unsupported call to `std::ptr::copy::<[u8; 4]>`
- fixtures/reject/src/lib.rs:1706:3: unsupported call to `std::ptr::copy_nonoverlapping::<[u8; 4]>`
- fixtures/reject/src/lib.rs:1707:14: unsupported call to `std::ptr::addr_eq::<[u8; 4], [u8; 4]>`
- fixtures/reject/src/lib.rs:1708:56: unsupported call to `std::ptr::mut_ptr::<impl *mut [u8; 4]>::add`
- fixtures/reject/src/lib.rs:1708:73: unsupported call to `std::ptr::const_ptr::<impl *const [u8; 4]>::offset`
- fixtures/reject/src/lib.rs:1712:58: unsupported call to `std::ptr::const_ptr::<impl *const [u8]>::len`

</details>

<details><summary>a raw pointer to `_` in a variable assigned again while it may point there (19)</summary>

- fixtures/reject/src/lib.rs:611:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:619:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:628:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:636:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:645:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:653:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:691:15: a raw pointer to `std::cell::UnsafeCell<u32>` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:704:15: a raw pointer to `std::cell::UnsafeCell<u32>` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:996:26: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1009:22: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1019:12: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1028:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1038:17: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1043:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1664:10: a raw pointer to `Target` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1677:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1692:10: a raw pointer to `(u32, u32)` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1728:26: a raw pointer to `[u8; 4]` in a variable assigned again while it may point there
- fixtures/reject/src/lib.rs:1729:10: a raw pointer to `[u8; 4]` in a variable assigned again while it may point there

</details>

<details><summary>a reference through which a raw pointer may be made into `_` in `_`, which the crate may put another object in place of (15)</summary>

- fixtures/reject/src/lib.rs:1332:33: a reference through which a raw pointer may be made into `InArray` in `[InArray; 2]`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1341:33: a reference through which a raw pointer may be made into `InTuple` in `(u16, InTuple)`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1350:34: a reference through which a raw pointer may be made into `Mid<InNested>` in `Outer<InNested>`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1371:33: a reference through which a raw pointer may be made into `Mid<InField>` in `Outer<InField>`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1384:35: a reference through which a raw pointer may be made into `Mid<InGeneric>` in `Outer<InGeneric>`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1393:35: a reference through which a raw pointer may be made into `InWritten` in `[InWritten; 1]`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1952:20: a reference through which a raw pointer may be made into `std::cell::UnsafeCell<u16>` in `Replaced`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1970:20: a reference through which a raw pointer may be made into `std::cell::UnsafeCell<u16>` in `Swapped`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:1988:20: a reference through which a raw pointer may be made into `std::cell::UnsafeCell<u16>` in `Taken`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:2007:20: a reference through which a raw pointer may be made into `std::cell::UnsafeCell<u16>` in `Assigned`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:2033:22: a reference through which a raw pointer may be made into `std::cell::UnsafeCell<u16>` in `Deeper`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:2057:20: a reference through which a raw pointer may be made into `std::cell::UnsafeCell<u16>` in `Dynamic`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:2073:15: a reference through which a raw pointer may be made into `std::cell::UnsafeCell<u16>` in `Closed`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:2086:20: a reference through which a raw pointer may be made into `std::cell::UnsafeCell<u16>` in `[std::cell::UnsafeCell<u16>; 2]`, which the crate may put another object in place of
- fixtures/reject/src/lib.rs:2132:20: a reference through which a raw pointer may be made into `Parked` in `Parking<T>`, which the crate may put another object in place of

</details>

<details><summary>a reference to `_`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there (15)</summary>

- fixtures/reject/src/lib.rs:1650:18: a reference to `std::cell::UnsafeCell<u64>`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1728:26: a reference to `[u8; 4]`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1746:22: a reference to `Behind`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1761:16: a reference to `Unboxed`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1782:18: a reference to `Deep`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1804:21: a reference to `Slotted`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1823:20: a reference to `Shared`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1840:20: a reference to `Inside`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1853:26: a reference to `Inside`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1868:22: a reference to `Compared`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1885:22: a reference to `Enclosing`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1901:17: a reference to `Chosen`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1914:19: a reference to `[u8; 4]`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:1929:21: a reference to `Wrapped<std::cell::Cell<u32>>`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there
- fixtures/reject/src/lib.rs:2103:13: a reference to `std::option::Option<Block<2>>`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there

</details>

<details><summary>`_` with a mutable reference to a value JS copies, which its mapping would copy (5)</summary>

- fixtures/reject/src/lib.rs:1230:22: `std::iter::once` with a mutable reference to a value JS copies, which its mapping would copy
- fixtures/reject/src/lib.rs:1632:13: `core::str::<impl str>::get_unchecked_mut` with a mutable reference to a value JS copies, which its mapping would copy
- fixtures/reject/src/lib.rs:1637:20: `std::str::from_utf8_mut` with a mutable reference to a value JS copies, which its mapping would copy
- fixtures/reject/src/lib.rs:2112:4: `std::collections::LinkedList::<T, A>::retain` with a mutable reference to a value JS copies, which its mapping would copy
- fixtures/reject/src/lib.rs:2114:2: `std::default::Default::default` with a mutable reference to a value JS copies, which its mapping would copy

</details>

<details><summary>a raw pointer to `_` in an element, a payload, or a union field, which is not supported yet (5)</summary>

- fixtures/reject/src/lib.rs:674:10: a raw pointer to `(u32, u32)` in an element, a payload, or a union field, which is not supported yet
- fixtures/reject/src/lib.rs:1721:15: a raw pointer to `[u8; 3]` in an element, a payload, or a union field, which is not supported yet
- fixtures/reject/src/lib.rs:1721:18: a raw pointer to `[u8; 3]` in an element, a payload, or a union field, which is not supported yet
- fixtures/reject/src/lib.rs:1721:37: a raw pointer to `[u8; 2]` in an element, a payload, or a union field, which is not supported yet
- fixtures/reject/src/lib.rs:1721:44: a raw pointer to `[u8; 2]` in an element, a payload, or a union field, which is not supported yet

</details>

<details><summary>a variant of `_`, which is not supported: either is the same JS string (4)</summary>

- fixtures/reject/src/lib.rs:331:2: a variant of `std::borrow::Cow<'_, str>`, which is not supported: either is the same JS string
- fixtures/reject/src/lib.rs:332:30: a variant of `std::borrow::Cow<'_, str>`, which is not supported: either is the same JS string
- fixtures/reject/src/lib.rs:333:27: a variant of `std::borrow::Cow<'_, str>`, which is not supported: either is the same JS string
- fixtures/reject/src/lib.rs:338:2: a variant of `std::borrow::Cow<'_, str>`, which is not supported: either is the same JS string

</details>

<details><summary>a copy of a closure that changes what it captured (3)</summary>

- fixtures/reject/src/lib.rs:224:14: a copy of a closure that changes what it captured
- fixtures/reject/src/lib.rs:509:14: a copy of a closure that changes what it captured
- fixtures/reject/src/lib.rs:510:14: a copy of a closure that changes what it captured

</details>

<details><summary>an assignment through a reference to `_` (3)</summary>

- fixtures/reject/src/lib.rs:83:2: an assignment through a reference to `std::sync::MutexGuard<'_, u8>`
- fixtures/reject/src/lib.rs:107:2: an assignment through a reference to `std::pin::Pin<std::boxed::Box<dyn std::future::Future<Output = u8>>>`
- fixtures/reject/src/lib.rs:214:2: an assignment through a reference to `std::pin::Pin<&mut Noisy>`

</details>

<details><summary>replacing the value behind a reference to `_` (3)</summary>

- fixtures/reject/src/lib.rs:91:2: replacing the value behind a reference to `std::pin::Pin<std::boxed::Box<std::future::PollFn<for<'a, 'b> fn(&'a mut std::task::Context<'b>) -> std::task::Poll<u8>>>>`
- fixtures/reject/src/lib.rs:99:2: replacing the value behind a reference to `std::pin::Pin<std::boxed::Box<[u8; 2]>>`
- fixtures/reject/src/lib.rs:112:2: replacing the value behind a reference to `std::sync::MutexGuard<'_, u8>`

</details>

<details><summary>unsupported type `_` (3)</summary>

- fixtures/reject/src/lib.rs:7:14: unsupported type `f16`
- fixtures/reject/src/lib.rs:7:25: unsupported type `f16`
- fixtures/reject/src/lib.rs:66:51: unsupported type `f16`

</details>

<details><summary>`_` is not supported: its `None` and `Some` would both be `rt.None` (2)</summary>

- fixtures/reject/src/lib.rs:16:15: `Option<std::option::Option<std::option::Option<u8>>>` is not supported: its `None` and `Some` would both be `rt.None`
- fixtures/reject/src/lib.rs:17:2: `Option<std::option::Option<std::option::Option<u8>>>` is not supported: its `None` and `Some` would both be `rt.None`

</details>

<details><summary>`_` of `_`, whose `None` is `rt.None`, which is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:37:4: `Iterator::scan` of `Option<std::option::Option<u8>>`, whose `None` is `rt.None`, which is not supported yet
- fixtures/reject/src/lib.rs:230:11: `Iterator::scan` of `Option<()>`, whose `None` is `rt.None`, which is not supported yet

</details>

<details><summary>`_` of `_`, whose `clone_from` is its own, which is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:21:4: `slice::fill` of `Reused`, whose `clone_from` is its own, which is not supported yet
- fixtures/reject/src/lib.rs:286:4: `slice::clone_from_slice` of `Reused`, whose `clone_from` is its own, which is not supported yet

</details>

<details><summary>`_`, a trait object of another crate's trait, which is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:11:16: `dyn std::iter::Iterator<Item = u8>`, a trait object of another crate's trait, which is not supported yet
- fixtures/reject/src/lib.rs:12:4: `dyn std::iter::Iterator<Item = u8>`, a trait object of another crate's trait, which is not supported yet

</details>

<details><summary>`_`, a trait object of the generic trait `_`, which is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:459:23: `dyn Scaled<u8>`, a trait object of the generic trait `Scaled`, which is not supported yet
- fixtures/reject/src/lib.rs:460:4: `dyn Scaled<u8>`, a trait object of the generic trait `Scaled`, which is not supported yet

</details>

<details><summary>a cast from a pointer to `_` to one to `_`, which reads the bytes of one type as another (2)</summary>

- fixtures/reject/src/lib.rs:550:27: a cast from a pointer to `u32` to one to `u8`, which reads the bytes of one type as another
- fixtures/reject/src/lib.rs:1708:17: a cast from a pointer to `[u8; 4]` to one to `u8`, which reads the bytes of one type as another

</details>

<details><summary>a coercion to `_`, which is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:450:40: a coercion to `std::sync::Weak<dyn Reading>`, which is not supported yet
- fixtures/reject/src/lib.rs:486:30: a coercion to `&Tail<dyn Reading>`, which is not supported yet

</details>

<details><summary>a raw pointer to a place of `()`, which has no address of its own (2)</summary>

- fixtures/reject/src/lib.rs:723:15: a raw pointer to a place of `()`, which has no address of its own
- fixtures/reject/src/lib.rs:723:29: a raw pointer to a place of `()`, which has no address of its own

</details>

<details><summary>an iterator of the crate implementing `_`, which is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:145:1: an iterator of the crate implementing `DoubleEndedIterator`, which is not supported yet
- fixtures/reject/src/lib.rs:193:1: an iterator of the crate implementing `FusedIterator`, which is not supported yet

</details>

<details><summary>comparing pointers to `_`, each a place JS makes of its own (2)</summary>

- fixtures/reject/src/lib.rs:591:2: comparing pointers to `u32`, each a place JS makes of its own
- fixtures/reject/src/lib.rs:2140:2: comparing pointers to `u32`, each a place JS makes of its own

</details>

<details><summary>overriding `_`, which std's adaptors call in place of `next` or beside it (2)</summary>

- fixtures/reject/src/lib.rs:125:2: overriding `Iterator::fold`, which std's adaptors call in place of `next` or beside it
- fixtures/reject/src/lib.rs:173:2: overriding `Iterator::size_hint`, which std's adaptors call in place of `next` or beside it

</details>

<details><summary>passing `_`, which drops, to `_`, which is not supported yet (2)</summary>

- fixtures/reject/src/lib.rs:326:4: passing `{closure@fixtures/reject/src/lib.rs:326:13: 326:27}`, which drops, to `str::contains`, which is not supported yet
- fixtures/reject/src/lib.rs:779:4: passing `{closure@fixtures/reject/src/lib.rs:779:11: 779:19}`, which drops, to `String::retain`, which is not supported yet

</details>

<details><summary>unsupported type `_` from another crate (2)</summary>

- fixtures/reject/src/lib.rs:433:32: unsupported type `std::net::AddrParseError` from another crate
- fixtures/reject/src/lib.rs:433:32: unsupported type `std::net::Ipv4Addr` from another crate

</details>

<details><summary>`Buf::copy_to_bytes` of `_`, a value JS copies, which is not supported (1)</summary>

- fixtures/reject/src/lib.rs:756:2: `Buf::copy_to_bytes` of `Blank`, a value JS copies, which is not supported

</details>

<details><summary>`BufMut::put` of `_` into a `BytesMut`, which picks the methods it calls by a capacity that depends on whether the other handles of its storage dropped, which is not supported (1)</summary>

- fixtures/reject/src/lib.rs:389:2: `BufMut::put` of `Counted` into a `BytesMut`, which picks the methods it calls by a capacity that depends on whether the other handles of its storage dropped, which is not supported

</details>

<details><summary>`Bytes::from_owner` of `_`, which would drop with the last clone of what it makes, which is not supported (1)</summary>

- fixtures/reject/src/lib.rs:374:2: `Bytes::from_owner` of `std::sync::Arc<[u8]>`, which would drop with the last clone of what it makes, which is not supported

</details>

<details><summary>`Extend` of `_` by `_`, whose `_` of `_` is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:1267:31: `Extend` of `(std::vec::Vec<u8>, Tens)` by `(u8, u8)`, whose `Tens` of `u8` is not supported yet

</details>

<details><summary>`Pointer` formatting, as in `{:p}`, is not supported (1)</summary>

- fixtures/reject/src/lib.rs:49:2: `Pointer` formatting, as in `{:p}`, is not supported

</details>

<details><summary>`_` is not supported: its `Pending` and `Ready` would both be `rt.OuterPending` (1)</summary>

- fixtures/reject/src/lib.rs:72:20: `Poll<std::option::Option<std::task::Poll<std::task::Poll<u8>>>>` is not supported: its `Pending` and `Ready` would both be `rt.OuterPending`

</details>

<details><summary>`_` is not supported: its field `_` is `_`, typed `_`, whose `None` would be `rt.None` as `_` (1)</summary>

- fixtures/reject/src/lib.rs:31:2: `Maybe<()>` is not supported: its field `value` is `Option<T>`, typed `T | undefined`, whose `None` would be `rt.None` as `Option<()>`

</details>

<details><summary>`_` is not supported: its field `_` is `_`, typed `_`, whose `Pending` would be `rt.OuterPending` as `_` (1)</summary>

- fixtures/reject/src/lib.rs:497:2: `Waiting<std::task::Poll<u8>>` is not supported: its field `poll` is `Poll<T>`, typed `rt.Poll<T>`, whose `Pending` would be `rt.OuterPending` as `Poll<std::task::Poll<u8>>`

</details>

<details><summary>`_` of `_` by `_`, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:253:6: `HashMap::contains_key` of `std::collections::HashMap<Id, u8>` by `u8`, which is not supported yet

</details>

<details><summary>`_` of `_`, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:533:4: `std::any::Any::type_id` of `dyn std::any::Any`, which is not supported yet

</details>

<details><summary>`_`, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:261:9: `Hasher::write`, which is not supported yet

</details>

<details><summary>`_`, which reads a capacity that depends on whether the other handles of its storage dropped, which is not supported (1)</summary>

- fixtures/reject/src/lib.rs:383:6: `BytesMut::capacity`, which reads a capacity that depends on whether the other handles of its storage dropped, which is not supported

</details>

<details><summary>`clone_from` of `_`, whose items' `clone_from` is their own, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:2113:4: `clone_from` of `std::collections::LinkedList<Reused>`, whose items' `clone_from` is their own, which is not supported yet

</details>

<details><summary>`collect` of `_`, which sorts by `_`'s own `Ord`, of entries that drop, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:950:22: `collect` of `std::collections::BTreeMap<std::cell::Cell<Ranked>, Noisy>`, which sorts by `std::cell::Cell<Ranked>`'s own `Ord`, of entries that drop, which is not supported yet

</details>

<details><summary>`fmt::Arguments` with a placeholder, or not made in place, written to `_`'s own `fmt::Write`, which is not supported yet: std splits what a placeholder writes into `write_str` calls the runtime does not make (1)</summary>

- fixtures/reject/src/lib.rs:55:10: `fmt::Arguments` with a placeholder, or not made in place, written to `Sink`'s own `fmt::Write`, which is not supported yet: std splits what a placeholder writes into `write_str` calls the runtime does not make

</details>

<details><summary>a `collect` in place of `_`, a zip std reads by index, holding values that drop, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:209:40: a `collect` in place of `std::iter::Map<std::iter::Zip<std::iter::Map<std::vec::IntoIter<u8>, {closure@fixtures/reject/src/lib.rs:208:32: 208:40}>, std::ops::Range<u8>>, {closure@fixtures/reject/src/lib.rs:209:24: 209:32}>`, a zip std reads by index, holding values that drop, which is not supported yet

</details>

<details><summary>a `skip` of `_`, a zip std reads by index, of items that drop, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:202:8: a `skip` of `std::iter::Zip<std::iter::Map<std::slice::Iter<'_, u8>, {closure@fixtures/reject/src/lib.rs:201:27: 201:31}>, std::iter::Map<std::slice::Iter<'_, u8>, {closure@fixtures/reject/src/lib.rs:201:59: 201:63}>>`, a zip std reads by index, of items that drop, which is not supported yet

</details>

<details><summary>a cast from `_` to `_` (1)</summary>

- fixtures/reject/src/lib.rs:68:2: a cast from `u64` to `f16`

</details>

<details><summary>a cast from `_` to `_`, which reads the bytes of one type as another (1)</summary>

- fixtures/reject/src/lib.rs:1708:33: a cast from `*const [u8; 4]` to `*const u8`, which reads the bytes of one type as another

</details>

<details><summary>a coercion of `_` to `_`, a pointer to a slice (1)</summary>

- fixtures/reject/src/lib.rs:1709:28: a coercion of `*const [u8; 4]` to `*const [u8]`, a pointer to a slice

</details>

<details><summary>a coercion to `_` (1)</summary>

- fixtures/reject/src/lib.rs:291:2: a coercion to `std::sync::Arc<[u8]>`

</details>

<details><summary>a constant of type `_`, a pointer into memory translation can't name (1)</summary>

- fixtures/reject/src/lib.rs:773:3: a constant of type `&u32`, a pointer into memory translation can't name

</details>

<details><summary>a dangling pointer to `_`, a value of no bytes made of more than structs and tuples (1)</summary>

- fixtures/reject/src/lib.rs:732:11: a dangling pointer to `Only`, a value of no bytes made of more than structs and tuples

</details>

<details><summary>a future of `_`, which can be `rt.Pending`, as the future's own `Pending` is (1)</summary>

- fixtures/reject/src/lib.rs:77:34: a future of `std::task::Poll<u8>`, which can be `rt.Pending`, as the future's own `Pending` is

</details>

<details><summary>a mapped guard of `_`, which JS can't write in place (1)</summary>

- fixtures/reject/src/lib.rs:2147:10: a mapped guard of `{closure@fixtures/reject/src/lib.rs:2146:38: 2146:50}`, which JS can't write in place

</details>

<details><summary>a mutable reference into a `Result`'s payload, passed on or kept (1)</summary>

- fixtures/reject/src/lib.rs:362:12: a mutable reference into a `Result`'s payload, passed on or kept

</details>

<details><summary>a mutable static, which any code may write (1)</summary>

- fixtures/reject/src/lib.rs:61:11: a mutable static, which any code may write

</details>

<details><summary>a pointer's address, which JS has no counterpart of (1)</summary>

- fixtures/reject/src/lib.rs:555:17: a pointer's address, which JS has no counterpart of

</details>

<details><summary>a printable type must implement `Display` or `Debug` (1)</summary>

- fixtures/reject/src/lib.rs:64:1: a printable type must implement `Display` or `Debug`

</details>

<details><summary>a raw pointer to `_` behind a reference, which is the object and not where it is kept (1)</summary>

- fixtures/reject/src/lib.rs:572:10: a raw pointer to `std::vec::Vec<u8>` behind a reference, which is the object and not where it is kept

</details>

<details><summary>a reference through which a raw pointer may be made into `_` in an element, a payload, or a union field, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:1359:11: a reference through which a raw pointer may be made into `InPayload` in an element, a payload, or a union field, which is not supported yet

</details>

<details><summary>a scalar constant of type `_` (1)</summary>

- fixtures/reject/src/lib.rs:8:2: a scalar constant of type `f16`

</details>

<details><summary>a trait object of `_`, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:476:26: a trait object of `[u8; 4]`, which is not supported yet

</details>

<details><summary>an upcast from `_` to `_`, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:527:32: an upcast from `dyn Keyed` to `dyn std::any::Any`, which is not supported yet

</details>

<details><summary>arithmetic on `_` (1)</summary>

- fixtures/reject/src/lib.rs:8:2: arithmetic on `f16`

</details>

<details><summary>comparing `_` (1)</summary>

- fixtures/reject/src/lib.rs:1707:41: comparing `*mut [u8; 4]`

</details>

<details><summary>comparing `_` as a key, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:235:75: comparing `std::collections::BTreeSet<u8>` as a key, which is not supported yet

</details>

<details><summary>comparing pointers to `_` in an atomic, which JS has no address of (1)</summary>

- fixtures/reject/src/lib.rs:772:15: comparing pointers to `u32` in an atomic, which JS has no address of

</details>

<details><summary>comparing pointers to `_`, which has no bytes and so no address of its own (1)</summary>

- fixtures/reject/src/lib.rs:723:70: comparing pointers to `Fused`, which has no bytes and so no address of its own

</details>

<details><summary>const generic types are not supported: `_` (1)</summary>

- fixtures/reject/src/lib.rs:2092:1: const generic types are not supported: `Block`

</details>

<details><summary>constructing `_` (1)</summary>

- fixtures/reject/src/lib.rs:349:53: constructing `std::borrow::Cow<'_, std::sync::Arc<u8>>`

</details>

<details><summary>hashing `_` as a key, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:235:75: hashing `std::collections::BTreeSet<u8>` as a key, which is not supported yet

</details>

<details><summary>hashing `_` by its address, which JS has no counterpart of (1)</summary>

- fixtures/reject/src/lib.rs:1711:5: hashing `*const [u8; 4]` by its address, which JS has no counterpart of

</details>

<details><summary>hashing `_` through `_`'s own `Hash::hash_slice`, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:310:16: hashing `std::vec::Vec<Batched>` through `Batched`'s own `Hash::hash_slice`, which is not supported yet

</details>

<details><summary>passing `_`, a closure that owns a value that drops, to `_`, which is not supported yet (1)</summary>

- fixtures/reject/src/lib.rs:319:13: passing `{closure@fixtures/reject/src/lib.rs:319:28: 319:35}`, a closure that owns a value that drops, to `Entry::or_insert_with`, which is not supported yet

</details>

<details><summary>union `_`, whose fields overlap in `_` and `f64`, which JS keeps apart (1)</summary>

- fixtures/reject/src/lib.rs:577:1: union `Bits`, whose fields overlap in `u32` and `f64`, which JS keeps apart

</details>
