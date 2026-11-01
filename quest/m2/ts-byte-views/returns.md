# [L] Byte slices passed and returned as parts

## Goal

A translated function outside the API boundary takes each byte slice parameter as its parts
(`data`, `start`, `length`), and one that returns a `Result` of byte slices and scalars, such
as `Result<(u64, &[u8]), E>`, returns the error or `undefined`, with its payload in a
module-level out slot. Chrome's varint and frame decode on `just ts bench` are faster than
`main`, no workload is meaningfully slower in any engine, and every differential test passes.

## Plan

Decided with the maintainer on 2026-10-28: this absorbs passing slices as parts, which
[#160](https://github.com/moq-dev/rust2/pull/160) implemented. Alone, parts gain nothing in
Chrome, since V8 already removes a slice object that doesn't escape once it inlines, and
Node's varint decode is 11-15% slower. They land only with the return ABI ("e1").
[#138](https://github.com/moq-dev/rust2/pull/138)'s per-function scratch measured slower
than `main`: copying between scratches when one function returns another's result cost the
most. One out slot per payload type, which a forwarded result already fills, avoids that.

- Only functions outside the API boundary: not roots or closures. A function used as a
  value is wrapped in an arrow that rebuilds the `Result`.
- The callee writes its slot at the JS `return`, after its MIR drops ran; the caller reads
  it at once, before any other call.
- Start the slot's number fields as `undefined`, not `0`. Such a field holds any value once
  a small integer is written (V8's `--trace-generalization`), so a write of a number that is
  no small integer, as an 8-byte varint is, boxes a new one into an old object. A field
  that starts as `0` stays a double written in place, but a slot that held one such number
  converts every small one read after: on Chrome, `p_0` starting at `0` took QUIC and
  leading-ones 8-byte varints from 1.13x and 1.24x `main`'s time to 0.88x and 1.06x, and
  frame decode from 0.83x and 0.67x to 0.90x and 0.76x (2026-10-29, interleaved, a busy
  host). Starting the starts and lengths at `0` changed nothing.
- Measured by hand edits of the codec's translation against `main`, Chrome first (2026-10-28,
  on a busy host, so confirm on a quiet one): with the slot's fields tagged, Chrome's varint
  and frame decode were each about 1.25x faster and group decode 1.06x, Node 1.33x and
  1.40x, Firefox 1.21x and 1.10x, Bun even and 1.12x. Leading-ones varints of 8 bytes
  measured 0.84x, within that run's noise.
- Decided with the maintainer on 2026-10-28: the translator writes the slot ("e1e"), its
  fields starting as `undefined`. Land only if Chrome's varint and frame decode both beat
  `main` on a quiet host, with the A/A route under 5% at the median and the max.
- Std's functions keep returning their `Result` whole, as their callers take it apart
  already. A slot sits beside the first function in source order that writes it, so its
  place doesn't depend on the order translation reaches them. `?`'s `Err` assigns a
  result held as its parts, so a function with early returns writes the slot at each.
- Measured from the translator's own output on 2026-10-29, four `just ts bench` runs against
  `main` at ccd52a0e. Chrome took 0.79-0.81 of `main`'s time on varint decode and 0.79-0.81
  on frame decode, and Node 0.68-0.78 and 0.71-0.72. No run's Chrome A/A came under 5% at the
  max: 5.5%, 61%, 16%, and 5.1%, with medians of 0.6-2.0%. It doesn't land on those runs.
- Open: Chrome's `time/u64` conversions took 1.11-1.17x `main`'s time in all four runs, with
  that workload's A/A within 4%, and `btree/` 1.08-1.28x in three of them. Swapping which
  route is `translated` kept `main` faster, so it isn't the route's place in the run. Their
  functions translate to the same text on both, and a harness of their own, running every
  workload in a fixed or shuffled order, timed them within 3% of `main`. So the gap is in
  how the bench drives them, not in their translation. Find which before landing.
- Leading-ones varints of 8 bytes took 1.06-1.24x `main`'s time: `read_leading_ones` is too
  big to inline, so each call boxes its value into the slot (above), where `main` returned
  a new array. Returning the `Ok`'s first number, rather than writing it to the slot, would
  avoid both the box and the conversion: an ABI change for the maintainer to weigh.
- Measured again on 2026-10-29 evening, other agents held, against `main` at ccd52a0e. CI
  jobs on the host's runners started during both deciding runs, and neither met the A/A
  rule: Chrome's was 4.0% at the median and 28% at the max, then 0.9% and 7.0%. Chrome took
  0.81x and 0.79x `main`'s time on varint decode, 1.01x and 0.81x on frame decode, 1.02x and
  1.03x on group decode, 1.18x and 0.84x on `btree/`, and 1.08x on `time/` conversions in
  both. Node took 0.68-0.74x on varint and 0.71x on frame decode.
- Timed alone, `time/` and `btree/` on the bench still put `time/u64` at 1.12-1.18x `main`,
  with the A/A within 6%, so the other workloads don't cause it. `btree/` swung from
  1.12-1.26x to 0.85-0.94x. One bench run under `--trace-turbo-inlining` showed this
  branch's `rescale` deopting 1,892 times, from Turbofan code compiled on stack replacement
  before the `put_u64` calls after its loop had run, where `main`'s deopted 28 times; but
  under `--trace-deopt` alone it deopted 4 and 7 times, so that loop came from the tracing.
- `rescale`'s only change from `main` is its `take_u64` calls: `take_u64` takes its slice
  as parts but makes it an `rt.ByteSlice` on entry, so each caller holding one splits it
  for the callee to make it again. That breaks this file's rule that nothing is made more
  often than before, but at 5 objects per 1024 conversions it can't be 8%. Not passing as
  parts a parameter its body makes on entry needs each callee's analysis before its
  callers are lowered. A hand edit of that (all outputs equal) deopted 0 and 1 times.
- Parked as a draft until the host is quiet, with its runners frozen. Then run the
  deciding bench, judged on family geomeans, and time the `take_u64` hand edit beside
  this branch, to see whether the call's shape is `time/`'s gap.

Public API: the translator's output for byte slices.
