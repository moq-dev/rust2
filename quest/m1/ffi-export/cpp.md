# [S] C++ bindings on MSVC

## Goal

The nightly's `Export moq-net to C++ (MSVC)` job passes: the exporter's C++ header passes
`just ffi test cpp` and `just ffi moq-cpp` with MSVC on Windows, as it does with GCC and
Clang on Linux and Apple Clang on macOS.

## Plan

- Read the next nightly's run, and fix what it finds at the source, in the generator or the
  recipes, never by loosening flags.
- Its runs so far stopped compiling `ffi/tests/fixtures.rs`, so nothing after that has run
  on Windows yet: the exporter, a rustc driver; cl's flags and its lookup of `link.exe`
  from Git Bash, where Git's `/usr/bin/link` may shadow it; the standin's paths; and
  MSVC's `/W4` warnings with no GCC or Clang counterpart.
