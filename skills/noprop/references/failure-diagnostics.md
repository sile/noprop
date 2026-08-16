# Failure Diagnostics

Read this file only after a noprop run fails or when code must inspect a
`RunError` programmatically.

## Reproduce the same failure

Copy the `reproduce with:` hint from the failure report and restore the
original property closure. Preserve all of the following:

- the printed seed;
- the original case budget; and
- the same property code and relevant external configuration.

The failure prints a `.run(N, |ctx| ...)` hint. Reusing the case budget also
preserves the run's rejection limit.

## Inspect `RunError`

| API | Meaning |
|-----|---------|
| `err.seed()` | Seed supplied to the failing runner. |
| `err.case_index()` | Zero-based accepted-case index associated with the failure. For too many rejections, the number of accepted cases completed before giving up. |
| `err.generated()` | Generated-value trace for the failing case, or the last rejected case for a too-many-rejections failure. |
| `err.stats()` | Counters recorded at failure. |
| `err.kind()` | `PropertyFailure` or `TooManyRejections`. |

Use `RunErrorKind` for control flow instead of matching formatted `Display` or
`Debug` text.

Both formatted representations contain the seed, failure message, reproduce
hint, statistics, and generated-value trace.

## Interpret `GeneratedValue`

Each value entry records the primitive sampler's call location, Rust type
name, and lazily formatted `Debug` value.

| API | Meaning |
|-----|---------|
| `type_name()` | Runtime Rust type name for the entry. |
| `location()` | Source location of the sampler call. |
| `value_repr()` | `Some(Debug text)` for a value entry; `None` only for an elision marker. |
| `is_elided()` | Whether the entry represents omitted repeated values from one call location. |
| `elided_count()` | Number of same-location generated-value entries omitted by that marker. |

Elision is output-size control for long runs of values produced at the same
source location. It does not mean that the sampled type lacked `Debug`, and
the count does not represent bytes, characters, or elements inside one value.

## Make the failure useful

Include semantic context in assertions: the step index, command, expected and
actual values, relevant model state, and enough command history to understand
the transition. Keep large histories bounded or summarize their irrelevant
prefix.

After reproducing the failure, reduce the witness by hand and add a focused
ordinary regression test. noprop does not provide automatic shrinking or an
on-disk failure corpus.
