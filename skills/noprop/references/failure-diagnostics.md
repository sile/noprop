# Failure Diagnostics

Read this file only after a noprop run fails or when code must inspect a
`RunError` programmatically. It covers reproduction, semantic diagnosis,
manual reduction, and freezing the shrunk witness as a regular `#[test]`.

## Three layers of failure information

Failure information falls into three layers with distinct roles. Keep them
separate; do not use one to substitute for another.

1. **Reproduction inputs** — `RunError::seed`, `RunError::case_index`, and
   the original case budget printed in the `reproduce with:` hint drive the
   next run. `RunError::stats` (`accepted_cases`, `rejected_cases`,
   `total_samples`) reports how far the failing run progressed; it is not
   an input to reproduction.
2. **Primitive trace** — `RunError::generated` records each `sample_*` call
   site and the value it produced during the failing case. It confirms
   what the sampler emitted at the leaf level. It does not preserve
   command order, assembled inputs, or model state — the same primitive
   value can mean different things depending on the property state that
   consumed it.
3. **Domain assertion message** — the string the property closure formats
   at the assertion site is the only layer that carries domain meaning:
   which command ran, the assembled input the SUT saw, model vs SUT
   state, and the observable consequence. This is where semantic diagnosis
   happens; do not offload domain context to the primitive trace.

## Reproduce the same failure

Copy the `reproduce with:` hint from the failure report and restore the
original property. Preserve all of the following:

- the printed seed;
- the original case budget (do not shrink to `case_index + 1`: the rejection
  cap is derived from the case budget, so shrinking it can turn a
  `PropertyFailure` into a `TooManyRejections`);
- the same property closure and generator code;
- the same relevant external configuration — env vars a `sample_*` call
  reads, fixtures a helper opens, feature flags, clock and I/O sources
  the closure touches.

The seed alone is not a guarantee. It reproduces the parts noprop
controls (its PRNG and case scheduling); a closure that reads the clock,
the process ID, or a network response still varies between runs. When
reproduction fails, first look for a hidden external input.

## Inspect `RunError`

| API | Meaning |
|-----|---------|
| `err.seed()` | Seed supplied to the failing runner. |
| `err.case_index()` | Zero-based accepted-case index associated with the failure. For too many rejections, the number of accepted cases completed before giving up. |
| `err.generated()` | Generated-value trace for the failing case, or the last rejected case for a too-many-rejections failure. |
| `err.stats()` | Progress counters recorded at failure. Not a reproduction input. |
| `err.kind()` | `PropertyFailure` or `TooManyRejections`. |

Use `RunErrorKind` for control flow instead of matching formatted `Display` or
`Debug` text. Both formatted representations contain the seed, failure
message, reproduce hint (with the original case budget embedded), stats,
and generated-value trace.

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

## Semantic assertion patterns

Assertion messages are the diagnosis layer. Format them so that a reader
who has only the failure report — no debugger, no rerun — can identify
the offending command, the assembled input, and the observable
consequence.

### Stateful command loops

For property that drive a model against a SUT with a command loop,
include in the assertion message:

- the step index, with the base explicit (0-based or 1-based);
- the command name and its salient arguments;
- the model state *before* the assertion (or the last transition the
  message is about — say which);
- the SUT state observed at the same moment;
- the expected value and the actual value the mismatch is on;
- the recent command history leading to the failure.

Bound the command history: never accumulate an unbounded `Vec<Command>`
across the run. Use a fixed-size ring buffer, "last N commands", or a
short prefix summary plus a detailed suffix. Choose a length that shows
the setup, the trigger, and the observable consequence.

Do not leave it ambiguous whether the state shown is pre- or post-command.
When several invariants fire during one command, name the invariant (or
the checked property) in the message so the reader knows which one
tripped.

```text
step 42: put(key=7, value=1) — after put, SUT.get(7) = Some(0), model.get(7) = Some(1)
last commands (5 most recent): get(3), put(7,0), get(7), get(9), put(7,1)
```

### Parser, scanner, serializer

For property that assemble input from smaller pieces (bytes, tokens,
escapes, chunks) and hand it to a parser or scanner, include:

- the final source the parser or scanner actually consumed;
- the ordered pieces or tokens the harness assembled;
- the parse or scan offset, line, and column (or byte offset if that is
  the addressable unit);
- the expected token or semantic value, and what the SUT produced.

Do not force the reader to reassemble the source in their head from the
individual `sample_char` or length draws in `GeneratedValue`.

Format large inputs lazily — inside the `assert!` message that only runs
on failure, or through a `Debug` wrapper with a length cap and an elision
marker. Unconditional `eprintln!` on every case will drown logs and pin
allocations. Never include secrets, credentials, or external data the
property is not authorized to log.

### Streaming and simulation

Codec, protocol, network, or storage simulations produce state and
progress that a per-command log cannot capture. Alongside the command
history, record:

- cumulative counts of bytes fed, bytes drained, records restored, and
  similar throughput quantities;
- bounded metrics — current queue length, pending operation count,
  active peers, retry attempts — where the *shape* of the number matters
  more than the exact value;
- notable boundaries the run crossed (`flush`, `finish`, `restart`,
  reconnect);
- a snapshot of the model state and the SUT state at the assertion
  point.

Do not dump entire internal state. Choose only the quantities that
distinguish the setup, the trigger, and the observable consequence of the
invariant that failed. When order matters (event streams, packet
sequences), record a bounded suffix rather than a count alone.

## Reduce and freeze as a regular regression test

After reproducing the failure and understanding the semantic root cause,
reduce the witness by hand and add a focused ordinary `#[test]`. A
seed-based property test is not a substitute for a regression test —
future generator tweaks can shift the seed's meaning; the frozen witness
does not.

Manual reduction. From the reproduced failure, ask in order:

1. Can any setup step be removed and the failure still fire? Remove
   prerequisites one at a time.
2. Can a prefix, suffix, or middle command be dropped from the command
   history?
3. Can any element be dropped from a collection, source, or payload?
4. Can any value be replaced with a domain boundary (0, empty, `MAX`)
   or a simple representative, and the failure still fire?
5. Does the final shrunk test still trigger the *same* observable
   consequence (same assertion, same message shape), or has it drifted
   to a different failure?

If step 5 no longer holds, the shrunk test proves a different bug — treat
that as a fresh finding, not as the minimization of the original.

Keep the seeded property test as well. The regression test guards the
specific witness; the property test keeps looking for related ones.

noprop does not provide automatic shrinking, on-disk failure persistence,
or a semantic log collector. Manual reduction plus the reproduce hint is
the intended workflow; if a real project keeps hitting a class of
failures where these are demonstrably insufficient, raise a concrete case
so the trade-off can be reconsidered on evidence.
