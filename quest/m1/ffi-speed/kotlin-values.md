# [S] Kotlin values without a LongArray

## Goal

A [rust2-ffi](/ffi/README.md) Kotlin binding holds a `Copy` value over 8 bytes, such as
moq's 16-byte `Timestamp`, in one object with no array beside it. Today it is a class
over a `LongArray`, two allocations per value, so
[the binding benchmark](/bench/ffi/README.md)'s stamped write costs 150 ns of CPU more
than the plain write in Kotlin (35 wall), against Swift's 1. The stamped write then costs
within its `from_micros` call of the plain write, as Go's and Dart's do.

## Plan

- One `Long` field per 8-byte word in the value's class, passed to and from JNI as that
  many `jlong` arguments and a returned struct's words written into the class's fields,
  so HotSpot's escape analysis can drop a value that does not escape.
- `equals` and `hashCode` keep today's rules, so only the storage changes.
- Re-measure the stamped write, CPU and wall, on HotSpot and on ART in the nightly
  emulator job.
