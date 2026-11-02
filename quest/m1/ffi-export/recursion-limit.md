# [S] Waiting: moq-tokio within the recursion limit

## Goal

`just ffi moq` exports moq's `main` again. [moq-dev/moq#7032](https://github.com/moq-dev/moq/pull/7032)
made moq-tokio's `tokio::spawn` of `moq_net::time::run` overflow the trait solver's
recursion limit proving `Send`, a future-compat lint on the exporter's nightly that the
recipe denies so a later nightly cannot break the export. Until moq fixes it, the
exporter's moq pin cannot move past it and misses moq's later API.

Check: on the exporter's nightly, in a moq checkout,
`RUSTFLAGS="-D recursion_depth_exceeding_limit" cargo check -p moq-tokio`. Advance it with a
quest in moq.

## Plan

Once it passes on moq's `main`, drop the exporter's own `moq_rev` for `just ts moq`'s, so the
translator and the exporter share one moq pin again, and update the config to what
`just ffi census` then refuses.
