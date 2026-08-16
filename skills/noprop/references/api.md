# noprop API Reference

Read this file when an exact public API, panic condition, or statistic is
needed. This snapshot targets unreleased noprop main (post-0.1.0).
Inspect the target project's resolved crate version and matching rustdoc
before relying on it outside the noprop repository.

## Execution

| API | Meaning |
|-----|---------|
| `Runner::new(seed: u64) -> Runner` | Construct a runner from a caller-supplied seed. |
| `runner.run(cases, property) -> RunResult` | Run uniform sampling until `cases` accepted cases complete or a failure occurs. |
| `runner.stats() -> Stats` | Return counters from the most recent run. |
| `TestCaseContext::new(seed: u64) -> TestCaseContext` | Construct a context directly for small experiments that do not need case rejection. |
| `seed_from_env_or_time(var: &str) -> TestResult<u64>` | Parse a seed from an environment variable or derive one from the clock when unset. |

The property has the shape
`Fn(&mut TestCaseContext) -> Result<(), Box<dyn Error>>`. A returned `Err` or a
panic fails the property. `TestResult<T = ()>` is the boxed-error alias used by
tests and properties; `RunResult` preserves `RunError` for runner calls.

## Statistics

| Field | Meaning |
|-------|---------|
| `accepted_cases` | Cases completed without case rejection. This reaches the requested case budget on success. |
| `rejected_cases` | Cases discarded directly or through exhausted bounded rejection. |
| `total_samples` | Top-level primitive sampler calls, including calls in rejected cases. |

Treat the internal rejection cap as an implementation detail. Inspect the
matching source when that value matters.

## Integers and ranges

| API | Output |
|-----|--------|
| `sample_bool(ctx)` | Uniform `bool`. |
| `sample_u8/u16/u32/u64/u128/usize(ctx)` | Uniform unsigned integer of the named type. |
| `sample_i8/i16/i32/i64/i128/isize(ctx)` | Uniform signed integer of the named type. |
| `sample_usize_in(ctx, range)` | Uniform `usize` inside any valid `RangeBounds<usize>`. |
| `sample_u64_in(ctx, range)` | Uniform `u64` inside any valid `RangeBounds<u64>`. |

Use `sample_usize_in(ctx, 0..n)` / `sample_u64_in(ctx, 0..n)` instead
of modulo reduction or bit masking. Invalid or empty ranges panic at the
caller.

## Bytes, characters, and strings

| API | Output |
|-----|--------|
| `sample_bytes::<N>(ctx)` | `[u8; N]`. |
| `sample_bytes_vec(ctx, len)` | `Vec<u8>` of exactly `len` bytes. |
| `sample_char(ctx)` | Any Unicode scalar value. |
| `sample_ascii_char(ctx)` | Any ASCII character, including controls. |
| `sample_ascii_printable_char(ctx)` | Printable ASCII from space through `~`. |
| `sample_string(ctx, len)` | `String` of exactly `len` Unicode scalar values. |
| `sample_ascii_string(ctx, len)` | ASCII string of exactly `len` bytes/code points. |
| `sample_ascii_printable_string(ctx, len)` | Printable ASCII string of exactly `len` bytes/code points. |

Draw a length separately when variable-size data is required.

## Floating point

| API | Output |
|-----|--------|
| `sample_f32(ctx)` / `sample_f64(ctx)` | A finite value over the full finite domain. |
| `sample_f32_in(ctx, min, max)` / `sample_f64_in(ctx, min, max)` | A value in the requested finite half-open range. |

Bounded float samplers panic unless both bounds are finite, `min < max`, and
`max - min` is also finite (a bound pair such as `f32::MIN..f32::MAX` overflows
the subtraction to infinity and cannot satisfy the finite-output contract).
Construct `f32::from_bits(sample_u32(ctx))` or
`f64::from_bits(sample_u64(ctx))` when arbitrary bit patterns, including NaN
and infinities, belong in the support.

## Choice and distribution

| API | Use |
|-----|-----|
| `sample_choice(ctx, choices)` | Pick one cloned value uniformly from a non-empty slice. The value must implement `Clone + Debug + 'static`. |
| `sample_weighted_index(ctx, weights)` | Pick an index in proportion to non-negative integer weights. |
| `sample_ratio(ctx, ratio)` | Return `true` with the exact rational probability. |
| `sample_with_boundaries(ctx, boundaries, ratio, sample)` | Select uniformly from non-empty boundary values with `ratio`; otherwise call the supplied sampler. |

`sample_weighted_index` panics on an empty slice, all-zero weights, or an
overflowing sum. `sample_choice` and `sample_with_boundaries` panic on empty
input.

`Ratio::new(numerator, denominator)` panics when the denominator is zero or
the numerator exceeds it. `Ratio::one_nth(n)` panics when `n` is zero. Both
constructors are `const fn`; validate runtime-derived values before calling
them.

## Rejection

| API | Scope |
|-----|-------|
| `sample_with_rejection(ctx, max_attempts, attempt)` | Retry one constrained draw until `attempt` returns `Some`, then reject the case on exhaustion. |
| `ctx.reject_case() -> !` | Reject the whole current case. |

`max_attempts == 0` panics. Both case-rejection paths require a surrounding
`Runner::run`; calling `reject_case` from a directly constructed context
panics.

For larger task-oriented examples, use the matching-version
`docs::recipes`, `docs::generator_authoring`, and `docs::generator_design`
modules from the crate rustdoc.
