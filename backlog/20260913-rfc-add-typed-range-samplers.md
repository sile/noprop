# RFC: Add typed range samplers for narrow integer fields

- Status: draft

## Summary

Add a small family of range samplers whose *result type* is the narrow integer
the caller actually wants, so `noprop::sample_u64_in(ctx, 1..=u16::MAX as u64)
as u16` becomes `noprop::sample_u16_in(ctx, 1..=u16::MAX)`. The existing
`sample_usize_in` / `sample_u64_in` stay as they are; this fills in the gaps
that today force a cast at every call site.

## Motivation

`sample_usize_in` and `sample_u64_in` return `usize` and `u64`. When the value
has to end up in a narrower field — a `u8` box version, a `u16` channel count, a
`u32` track id, an `i32` difference — callers write a widening cast on the
range and a narrowing cast on the result. The narrowing cast is the problem:
it is unchecked trivia at every call site, and it silently truncates if the
range is ever edited to exceed the target type.

This was observed at scale in a real consumer (shiguredo/mp4-rs, `pbt/`),
migrated from proptest to noprop in 0.2. There are on the order of 250 call
sites of the shape:

```rust
let es_id = noprop::sample_u64_in(ctx, 1..=u16::MAX as u64) as u16;
let stream_priority = noprop::sample_u64_in(ctx, 0..32) as u8;
let length_size = noprop::sample_u64_in(ctx, 0..4) as u8;
noprop::sample_u64_in(ctx, 0x60..=0x7F) as u8,
```

Two things stand out:

- The bound is written in the *target* type and then widened (`1..=u16::MAX as
  u64`), because the caller is thinking in terms of the field, not in terms of
  `u64`. The cast on the bound is pure noise.
- The result is narrowed immediately. That cast is where a mistake hides: if
  someone changes the range to `1..=(u16::MAX as u64 + 1)` the code still
  compiles and truncates.

A typed result removes both casts and makes the range and the field share one
type, so an out-of-range bound is a compile error rather than a silent wrap.

The upstream issue that scoped this (a survey of ten noprop consumers) recorded
the decision to add only these typed samplers to the core and to leave vector,
option, runner-shape, string and boundary helpers to user-side recipes. See
"Rationale and alternatives" for why this candidate and not the others.

## Guide-level explanation

Today, a property that builds a field of a known width has to launder the value
through a wider type:

```rust
// before
let version = noprop::sample_u64_in(ctx, 0..2) as u8;
let channel_count = noprop::sample_u64_in(ctx, 1..=8) as u16;
let track_id = noprop::sample_u64_in(ctx, 1..=1_000) as u32;
```

After this change the type flows straight through:

```rust
// after
let version = noprop::sample_u8_in(ctx, 0..2);
let channel_count = noprop::sample_u16_in(ctx, 1..=8);
let track_id = noprop::sample_u32_in(ctx, 1..=1_000);
```

There is nothing new to learn beyond the naming rule that already exists:
`sample_<type>_in(ctx, range)` samples a `<type>` inside `range`, exactly as
`sample_u64_in` does today. The only change is that the family now covers the
types whose ranges are awkward to express some other way.

For signed values, the same idea removes an offset trick users otherwise have
to invent:

```rust
// before: sample unsigned, then offset into the signed domain
let diff = noprop::sample_u64_in(ctx, 0..=255) as i32 - 128;

// after
let diff = noprop::sample_i32_in(ctx, -128..=127);
```

## Reference-level explanation

Add four public functions to `src/generator.rs`, following the implementation
pattern of `sample_usize_in` / `sample_u64_in` exactly (bound normalization,
empty-range panic, full-width fast path, `ctx.record_generated`, and
`#[track_caller]`).

```rust
#[track_caller]
pub fn sample_u8_in<R: RangeBounds<u8>>(
    ctx: &mut TestCaseContext,
    range: R,
) -> u8;

#[track_caller]
pub fn sample_u16_in<R: RangeBounds<u16>>(
    ctx: &mut TestCaseContext,
    range: R,
) -> u16;

#[track_caller]
pub fn sample_u32_in<R: RangeBounds<u32>>(
    ctx: &mut TestCaseContext,
    range: R,
) -> u32;

#[track_caller]
pub fn sample_i32_in<R: RangeBounds<i32>>(
    ctx: &mut TestCaseContext,
    range: R,
) -> i32;
```

Design points:

- **Bounds are the result type.** `u8` for `sample_u8_in`, and so on. The
  accepted range forms are, as elsewhere, any `RangeBounds`: `a..b`, `a..=b`,
  `..b`, `a..`, and `..`.
- **Panic on empty range**, with a `#[track_caller]` message naming the
  function (`sample_u8_in: empty range`), matching the existing messages.
- **Uniform, unbiased sampling.** Implement by delegating to the existing
  64-bit sampler over the widened range and narrowing the result, or by the
  same rejection path. Either way, a bound cast must be provably lossless: the
  widened width is exact because the source type is narrower than `u64`, and
  the narrowing back is exact because the sampled value is already inside the
  type's range. No truncation is possible by construction.
- **Determinism is not promised across these functions.** As the existing
  `sample_u64_in` documentation already warns, the byte count consumed depends
  on the range width, so the exact number of bytes drawn is an implementation
  detail. A user who wants the same RNG stream should use the same sampler.
- **Signed support via an offset, not a separate algorithm.** `sample_i32_in`
  can be implemented as sample-in-`0..=width` plus `lo` (with unsigned
  arithmetic under the hood), so signedness does not require a new sampling
  core. Choose an implementation that keeps the full `i32::MIN..=i32::MAX`
  range correct, including the unbounded (`..`) case.

Scope: unsigned `u8` / `u16` / `u32` and signed `i32` only. This is the set
that the motivating consumer needs; see "Unresolved questions" for the
types left out.

## Drawbacks

- **More API surface on a 0.2 crate.** Four functions are four more names that
  become part of the public surface and, realistically, have to keep working.
  The `sample_*` family is already large; this makes it larger.
- **Overlap with existing functions.** `sample_usize_in` already returns a type
  wide enough for `u8` / `u16` / `u32` values on 32/64-bit targets, so there is
  a redundancy: a caller *can* write `sample_usize_in(ctx, 0..2) as u8`. The
  new functions are for ergonomics and for the compile-time range check, not
  because the old ones are wrong.
- **Another naming pattern to remember.** The rule "the digits in the name are
  the result type" is easy, but it is the third such family member after
  `sample_usize_in` and `sample_u64_in`, so the doc on each should point at the
  others to avoid the appearance of arbitrary coverage.

## Rationale and alternatives

This candidate was selected from a survey of ten projects that consume noprop.
The alternatives below are recorded here so the same ground is not re-argued.

- **Do nothing.** Callers keep the two-cast idiom. It is not broken, only
  noisy, and it is the current state of a real 250-call-site consumer. The
  cost of doing nothing is that the unchecked narrowing cast stays everywhere,
  so a later range edit can silently truncate.
- **`sample_vec` / collection helpers (rejected for core).** A consumer had the
  same `fn sample_vec<T>` copied byte-for-byte into eleven files. But two other
  consumers had already solved it by defining *one* `sample_vec` in a shared
  harness module (`tests/pbt_harness.rs` / `pbt/src/lib.rs`) and sharing it
  across tests. The duplication disappears by choosing a place for the helper,
  not by putting it in the crate. Documented as a recipe, not added here.
- **Runner-shape helper / `run_test` (rejected for core).** Every project names
  its seed environment variable differently (`CONTAINER_RS_SEED`,
  `HTTP2_PBT_SEED`, `NORAFT_PBT_SEED`, ...), so a core helper would have to
  take the name as a parameter and would save only a couple of lines. One
  project's `run_config` and another's harness module are the models to copy;
  a recipe covers this.
- **`sample_option` (rejected for core).** Re-implemented in several consumers
  but small and locally defined; a shared harness handles it, as with
  `sample_vec`.
- **String / regex helpers (rejected for core).** The character set is
  project-specific, so a core function would have to be generic over the
  alphabet and ends up saving little.
- **Boundary and length combinators (rejected for core).** The most duplicated
  pattern across consumers, but the set of boundary values is project-specific,
  so it belongs in a user-side helper.
- **Why these four types and not a single generic `sample_in<T>`.** A
  `RangeBounds<T>` generic over an integer trait cannot be written today: Rust
  has no integer trait that covers `u8` / `u16` / `u32` / `i32` and is usable as
  a `RangeBounds` element, and the existing family is already one function per
  type. Naming each type keeps the pattern consistent and avoids waiting on a
  language feature.
- **Why not `i8` / `i16` / `u128`, etc.** See "Unresolved questions".

## Unresolved questions

- Which signed types to include. `i32` is chosen because the motivating
  consumer needs `i32`; `i8` and `i16` are plausible and cheap, but adding them
  without a call site would be speculative. Decide whether to add them now for
  symmetry with the unsigned set or defer until needed.
- Whether `sample_u128_in` is ever wanted. The existing `sample_u64_in` doc
  explicitly declines it for lack of demand; that reasoning still holds unless
  a consumer asks.
- Whether the doc on `sample_u64_in` (which today explains "why no other
  integer `_in` variants") should be rewritten once this RFC lands, since its
  premise becomes obsolete. This is a documentation change that should land in
  the same commit as the code, or explicitly be left for a follow-up.

## Future possibilities

- If `i8` / `i16` are added later, the pattern is mechanical and can follow the
  same shape.
- A future language feature (an integer trait usable with `RangeBounds`) could
  collapse the whole family into one generic function; the per-type names are
  forward-compatible with that because a generic function can be introduced
  alongside them and the typed names retained as thin wrappers.
- The recipe side of the same survey (vector / option / runner-shape / string /
  boundary helpers) is out of scope here and belongs in `docs/recipes.md`.
