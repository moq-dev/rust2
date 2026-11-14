# [M] Export moq-tokio's init_android on Android

## Goal

`moq_tokio::tls::init_android(env: &mut jni::Env, context: JObject)`, which exists only on
Android, exports there, so `ffi/moq/ffi.toml` drops its exclude and the Android staging of
`just ffi moq-api` refuses nothing. Its `jni` types match no rule in
[rust2-ffi](/ffi/README.md); without the exclude, the export follows them into the whole
`jni` crate and refuses 185 items.

## Plan

`tls::Error::AndroidInit(jni::errors::Error)` needs nothing: `tls::Error` crosses as an
opaque error with its message, so its variants are no part of the binding.

moq-ffi calls `init_android` today from its own `JNI_OnLoad`, with the application
`Context` it finds through `ActivityThread.currentApplication()`, so apps need no setup. The
generated Kotlin glue defines `JNI_OnLoad` too, so the two cannot share a library once the
real moq-ffi replaces the stand-in.

Recommended: a general rule, that a `pub fn` taking a `&mut jni::Env` exports to Kotlin
alone, the glue passing the env of the calling thread, and a `JObject` crosses as Kotlin's
`Any`. Kotlin then calls `tls.initAndroid(context)`, which apps must call themselves unless moq's
Kotlin calls it for them.

## Waiting

- Maintainer: how `init_android` reaches Kotlin. Options: the general rule above
  (recommended: configless, and serves any crate's JNI entry point); the glue calling
  `init_android` itself when the library loads, as the 2026-11-12 audit recommended (keeps
  apps setup-free, but names one moq function in the exporter); or moq keeping
  `init_android` out of its `pub` API and calling it from code the glue's `JNI_OnLoad` runs.
