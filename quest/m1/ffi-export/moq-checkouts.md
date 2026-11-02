# [S] A moq checkout per pin

## Goal

`just ts` and `just ffi` each check out the moq revision they pin in a directory of its own,
so running both at once in one worktree, such as `just ci` beside `just ffi bench`, builds
each against its own revision.

## Plan

Found by [#407](https://github.com/moq-dev/rust2/pull/407): `just ffi standin` reads
`ts/target/moq`, which `just ts moq-pin` checks out at ts's revision, and the two pins have
differed since [#397](https://github.com/moq-dev/rust2/pull/397). Running both at once fails
the export with a missing crate. Key the checkout by its revision, or give ffi its own,
sharing the clone's objects rather than fetching twice.

Public API: none.
