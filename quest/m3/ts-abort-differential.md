# [S] Aborts in the differential

## Goal

The lang differential compares a wasm32 abort with the translation's, so the runtime's
abort of an `RwLock` locked again while held, `rwlock locked for reading` or `for writing`,
is tested as `RefCell`'s panics are. Today wasm32's `rtabort!` traps with no panic
message, which no translation's `rt.Panic` matches, so the fixtures avoid it.

## Plan

Tell an abort from a panic on both sides: natively by the trap without a panic hook's
message, and in the runtime by a distinct error or message prefix such as std's
`fatal runtime error: ..., aborting`. Keep the comparison exact for panics.
