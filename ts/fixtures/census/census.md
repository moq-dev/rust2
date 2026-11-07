# Census of `census-fixture`

- Refusals: 9
- Function instances: 6
- Generic functions no instance reached, lowered over their type parameters: 1

## By construct

| Count | Construct | First |
| ---: | --- | --- |
| 3 | a cast from `_` to `_`, between a pointer and an integer | fixtures/census/src/lib.rs:9:2 |
| 2 | a cast from a pointer to `_` to one to `_`, which reads the bytes of one type as another | fixtures/census/src/lib.rs:19:12 |
| 2 | a mutable static, which any code may write | fixtures/census/src/lib.rs:38:3 |
| 1 | `Pointer` formatting, as in `{:p}`, is not supported | fixtures/census/src/lib.rs:24:2 |
| 1 | union `_`, whose fields overlap in `_` and `f64`, which JS keeps apart | fixtures/census/src/lib.rs:28:1 |

## By API

The type or module each refused type or path belongs to.

| Count | API | First |
| ---: | --- | --- |
| 3 | `usize` | fixtures/census/src/lib.rs:9:2 |
| 2 | `u32` | fixtures/census/src/lib.rs:19:12 |
| 2 | `u8` | fixtures/census/src/lib.rs:19:12 |
| 1 | `*const T` | fixtures/census/src/lib.rs:14:2 |
| 1 | `*const bytes::Bytes` | fixtures/census/src/lib.rs:45:2 |
| 1 | `*const u32` | fixtures/census/src/lib.rs:9:2 |
| 1 | `Bits` | fixtures/census/src/lib.rs:28:1 |
| 1 | `u64` | fixtures/census/src/lib.rs:19:47 |

## By type or path

| Count | Construct | Type or path | First |
| ---: | --- | --- | --- |
| 1 | a cast from `_` to `_`, between a pointer and an integer | `*const T`, `usize` | fixtures/census/src/lib.rs:14:2 |
| 1 | a cast from `_` to `_`, between a pointer and an integer | `*const bytes::Bytes`, `usize` | fixtures/census/src/lib.rs:45:2 |
| 1 | a cast from `_` to `_`, between a pointer and an integer | `*const u32`, `usize` | fixtures/census/src/lib.rs:9:2 |
| 1 | a cast from a pointer to `_` to one to `_`, which reads the bytes of one type as another | `u32`, `u8` | fixtures/census/src/lib.rs:19:12 |
| 1 | a cast from a pointer to `_` to one to `_`, which reads the bytes of one type as another | `u64`, `u8` | fixtures/census/src/lib.rs:19:47 |
| 1 | union `_`, whose fields overlap in `_` and `f64`, which JS keeps apart | `Bits`, `u32` | fixtures/census/src/lib.rs:28:1 |

## Spans

<details><summary>a cast from `_` to `_`, between a pointer and an integer (3)</summary>

- fixtures/census/src/lib.rs:9:2: a cast from `*const u32` to `usize`, between a pointer and an integer
- fixtures/census/src/lib.rs:14:2: a cast from `*const T` to `usize`, between a pointer and an integer
- fixtures/census/src/lib.rs:45:2: a cast from `*const bytes::Bytes` to `usize`, between a pointer and an integer

</details>

<details><summary>a cast from a pointer to `_` to one to `_`, which reads the bytes of one type as another (2)</summary>

- fixtures/census/src/lib.rs:19:12: a cast from a pointer to `u32` to one to `u8`, which reads the bytes of one type as another
- fixtures/census/src/lib.rs:19:47: a cast from a pointer to `u64` to one to `u8`, which reads the bytes of one type as another

</details>

<details><summary>a mutable static, which any code may write (2)</summary>

- fixtures/census/src/lib.rs:38:3: a mutable static, which any code may write
- fixtures/census/src/lib.rs:39:3: a mutable static, which any code may write

</details>

<details><summary>`Pointer` formatting, as in `{:p}`, is not supported (1)</summary>

- fixtures/census/src/lib.rs:24:2: `Pointer` formatting, as in `{:p}`, is not supported

</details>

<details><summary>union `_`, whose fields overlap in `_` and `f64`, which JS keeps apart (1)</summary>

- fixtures/census/src/lib.rs:28:1: union `Bits`, whose fields overlap in `u32` and `f64`, which JS keeps apart

</details>
