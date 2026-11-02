# [XS] No unused import in an export's generated crate

## Goal

The crate rust2-ffi generates builds without warnings when no export takes or returns a
`void *`, as an export of plain functions over bytes does.

## Plan

Found by [#410](https://github.com/moq-dev/rust2/pull/410): exporting the Go fixture's
codec, whose API is two functions over bytes, warns `unused import: std::ffi::c_void`.
`Glue::lib` (`ffi/src/glue.rs`) writes the import unconditionally; write it only when the
generated code names `c_void`, and have the fixture test fail on any warning in the
generated crate.

Public API: none.
