# Generator Design

This document explains the small set of decisions every noprop
generator has to make. It is a working reference for users writing
their own `sample_*` helpers, not a research plan or a roadmap for
future tuning. For the how-to side (composing primitives, bounded
rejection, `NonZero` recipes, floats), see
[`crate::docs::generator_authoring`].

## Support

The **support** of a generator is the set of values it can produce.
noprop generators are `fn(&mut TestCaseContext) -> T`, so the support
is whatever `T` values the body may return.

Two practical rules:

- **Say what the support is, in the docstring.** "Any `u32`", "any
  non-empty ASCII printable string up to `len` bytes", "any
  `NonZeroU32`". A generator whose support is fuzzy — undocumented
  edge cases, silently-excluded values — hides bugs when the property
  fails on an input the caller believed impossible.
- **Match the domain in the return type.** Prefer `NonZeroU32` over
  `u32` with a `!= 0` postcondition, `Vec<u8>` with a documented length
  bound over "usually short", and a small `enum` over a magic-number
  `u8`. Restricting the type restricts the support at the type system,
  so the property does not have to re-check what the generator already
  guaranteed.

## Distribution

A generator's **distribution** is *how* it samples values across the
support. noprop's primitives are uniform over their support:
`sample_u32` covers `0..=u32::MAX` uniformly, `sample_usize_in(ctx,
0..=n)` covers `0..=n` uniformly (through bias-free bounded rejection),
and `sample_choice(ctx, slice)` picks one slice element uniformly.

Two reasons to move away from uniform:

- **Bias toward interesting inputs.** Domain boundaries (0, one, the
  buffer size, `u16::MAX`) are hit by uniform sampling with vanishing
  probability. Use [`sample_with_boundaries`](crate::sample_with_boundaries)
  to mix a small set of interesting candidates in at an exact rational
  probability (`Ratio::one_nth(10)`, `Ratio::one_nth(100)`, …). The
  underlying uniform draw is unchanged; only the mixing weight is new.
- **Weighted branch selection.** A `match` on
  [`sample_weighted_index`](crate::sample_weighted_index) picks a
  branch by a caller-specified weight vector, so rare branches can be
  exercised without paying uniformly for every one. Prefer this over
  ad-hoc `if sample_ratio(...)` chains: the weights are visible in one
  place and the RNG cost is a single draw.

Every distribution decision is exact rational, not floating point:
`Ratio::new(1, 3)` is one-in-three, not `0.333…`. This keeps the
sampled distribution identical across platforms and reproducible from a
seed.

## Termination

Every generator noprop provides terminates in a finite number of RNG
draws for any seed. Two constructs risk violating that guarantee if
users write their own:

- **Unbounded `loop { … }` retries.** A hand-written "keep drawing
  until the predicate holds" wedge on choice sequences where every
  draw fails. Use
  [`sample_with_rejection`](crate::sample_with_rejection) instead —
  it enforces a `max_attempts` bound and, on exhaustion, rejects the
  enclosing case rather than looping forever.
- **Unbounded recursion.** A generator that calls itself without a
  decreasing depth argument (or an equivalent guard) can run without
  bound. Take an explicit depth (or count) parameter, decrease it in
  the recursive call, and return a base-case value when it hits zero.

Internally, noprop's shared bounded-domain sampler — the crate-private
core used by [`sample_usize_in`](crate::sample_usize_in),
[`sample_ratio`](crate::sample_ratio),
[`sample_weighted_index`](crate::sample_weighted_index),
[`sample_choice`](crate::sample_choice), and
[`sample_with_boundaries`](crate::sample_with_boundaries) — is bounded
at 64 attempts per call. The per-attempt rejection rate is at worst ~50%
(when the requested domain is just above a power of two), so the
probability of exhausting all 64 attempts is `< 2⁻⁶⁴` — astronomically
unreachable in practice. If it does trigger inside `Runner::run`, the
current case is rejected (via
[`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case));
outside a runner the call panics with a Runner-only message.

## Rejection scope

Rejection discards a value. noprop has two rejection scopes, and the
choice between them is a design decision the caller has to make:

- **Single-draw rejection** with
  [`sample_with_rejection`](crate::sample_with_rejection). Retries a
  local constrained draw (e.g. "an identifier: ASCII alphabetic or
  underscore, not starting with a digit") up to `max_attempts` times;
  only that draw is redrawn, not the surrounding case. This is the
  right scope when the constraint is a property of the value being
  drawn.
- **Whole-case rejection** with
  [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case).
  Discards the entire case as unsuitable after sampling has already
  finished (e.g. "this generated config line contains a comment, so it
  is not a record"). Rejected cases are retried and do not count toward
  the `cases` budget. This is the right scope when the precondition
  spans the whole case.

Total rejections across a run are bounded by an internal, `cases`-scaled
cap; a generator that always rejects still terminates with a
`TooManyRejections` failure. The cap is not a public knob; it is
generous relative to any realistic accept rate.

## Valid-by-construction

The best rejection scope is often *no rejection at all*. A
valid-by-construction generator picks a value that already satisfies
the constraint, instead of sampling widely and discarding failures.

Two typical shapes:

- **Sample the constraining parameter first, then satisfy it.** Draw
  the length with `sample_usize_in(ctx, 0..=max)`, then call
  `sample_bytes_vec(ctx, len)` or `sample_string(ctx, len)`. The
  result always has a length in range; there is no length rejection.
- **Compose within the domain.** For a `NonZeroU32`, either use
  bounded rejection over `sample_u32` (uniform, may reject) or map the
  underlying integer's zero to `1` explicitly (biased, always
  terminates in one draw). The two recipes are spelled out in the
  "Sampling non-zero integers" section of
  [`crate::docs::generator_authoring`] — pick the one whose trade-off
  matches the property.

When the constraint is expensive to satisfy at construction time —
e.g. "any two lists that share at least one element" — rejection is
often the pragmatic choice. Note when the accept rate is low
(≤ ~10%) so the caller can enlarge `max_attempts` accordingly.

## Recording the value, not just the byte source

Every user-defined `sample_*` should record its produced value into
the trace so failures show what the generator returned, not the raw
byte source. The primitives (`sample_u32`, `sample_string`, …) already
record; wrappers only need to record when they transform the value:

```rust
use std::num::NonZeroU32;
use noprop::TestCaseContext;

#[track_caller]
fn sample_non_zero_u32(ctx: &mut TestCaseContext) -> NonZeroU32 {
    let v = noprop::sample_u32(ctx);
    NonZeroU32::new(if v == 0 { 1 } else { v })
        .expect("v was remapped away from zero")
}
```

`#[track_caller]` puts the trace entry at the user's call site, not
inside the wrapper — so a failure trace points at where the caller
asked for the value.
