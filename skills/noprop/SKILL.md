---
name: noprop
description: >
  Implement, review, debug, and explain Rust property tests that use the
  noprop crate. Use when a task names noprop, contains `noprop::` APIs or a
  Cargo dependency on noprop, or asks specifically about noprop's Runner,
  TestCaseContext, Ratio, `sample_*`, rejection, or failure reproduction.
  Do not activate for generic Rust property-testing tasks that use another
  framework.
---

# noprop

Use noprop to write imperative property tests as plain Rust closures. Focus
first on making the intended bug region reachable and likely to be explored;
API selection and case count come after the search space is sound.

## Follow this workflow

### 1. Establish the applicable API

- Treat the guidance in this skill as targeting noprop 0.2.0.
- Inspect the target project's `Cargo.toml` and `Cargo.lock`; when it resolves
  a different version, use that version's matching rustdoc instead.
- When changing noprop itself, treat `src/lib.rs`, the public rustdoc, and the
  relevant root `docs/*.md` file as the source of truth.
- When changing a consuming project, do not silently upgrade its noprop API.
- Read [references/api.md](references/api.md) only when an exact signature,
  sampler, panic condition, or statistic is needed.

### 2. State what the property must explore

Write down the following before writing generators:

1. Name the invariant or oracle: model-versus-SUT, differential,
   round-trip, metamorphic relation, or a domain invariant.
2. Identify the behavior that could falsify it, including its setup,
   trigger, and observable consequence.
3. Identify boundary values, dependent values, preconditions, state
   transitions, and sequence lengths needed to reach that behavior.
4. Decide what evidence will prove that the meaningful region was actually
   exercised. Add a coverage gate when the invariant can otherwise pass
   vacuously.

Do not start by drawing convenient primitive types. Start from the semantic
states and operations the test must reach, then map them onto noprop draws.

### 3. Design an effective search space

For every generator, decide its support, distribution, and termination.

- Make the support contain every class relevant to the property, including
  empty, singleton, maximum supported size within the test's explicit budget,
  invalid-input, and exceptional classes when they belong to the SUT's public
  input domain.
- Represent constrained domains with types and valid-by-construction values.
  Draw a length before its payload, a protocol version before its dependent
  fields, and consult model state before choosing a legal command.
- Keep loops and recursion explicitly bounded. Sample the bound or depth from
  a finite range and include both short and long cases deliberately.
- Use `sample_with_boundaries` to give domain boundaries meaningful
  probability without discarding the interior distribution.
- Use `sample_weighted_index` for branch or command selection. Keep the
  weights visible in one place and ensure prerequisite operations, trigger
  operations, and observation operations all receive enough probability.
- Prefer state-dependent command selection over generating mostly illegal
  commands and rejecting them. Generate invalid commands intentionally only
  when invalid-command behavior is part of the property.
- Prefer operation sequences over arbitrary final states when the bug depends
  on history. Check the model and invariant after each meaningful transition;
  defer checks to finalization only when the API semantics require it.

Treat increasing `cases` as the last tuning lever. More cases do not repair a
support hole, an unreachable command sequence, a vacuous invariant, or a
distribution that assigns negligible mass to the target region.

### 4. Use rejection at the narrowest scope

- Prefer valid-by-construction generation when a constraint can be expressed
  directly.
- Use `sample_with_rejection(ctx, max_attempts, ...)` for one constrained
  draw. Choose `max_attempts` from the expected acceptance rate rather than
  copying an arbitrary value.
- Use `ctx.reject_case()` only when a precondition depends on the completed
  case and the whole case is unsuitable.
- Rework the generator when rejection is common. Frequent rejection wastes
  the case budget and usually indicates that the sampled representation is
  broader than the intended domain.
- Never write an unbounded retry loop.

### 5. Prevent vacuous success

Place assertions where the relevant behavior occurs, and separately count
whether that location was reached. After the run, fail if the count is zero.
Coverage gates use interior mutability because the property closure implements
`Fn`; choose the cell shape by what the gate means:

- `Cell<bool>` for "did we ever reach it".
- `Cell<usize>` for count- or rate-based gates.
- `RefCell<T>` only when the gate needs a non-`Copy` aggregate across cases
  (a bounded history of witnesses, a set of reached buckets).
- Per-case temporaries live in a plain `let mut` inside the closure, not in
  interior mutability.

Increment the cell at the invariant-eval site — not where the target value
was drawn, not where a branch was selected. Order the closure so every
`ctx.reject_case()` and every `sample_with_rejection` exit sits strictly
before any gate update; a case that increments a counter and then gets
rejected leaves discarded evidence in the count. Do not treat attempt count
or rejected-case count as proof that the invariant ran.

Assert each gate individually and include `{runner}` in the message
(`Runner` implements `Display` but not `Debug`; `{runner:?}` will not
compile). When both sides of a branch matter (empty vs non-empty, success vs
error), pair the counters and assert both. Keep
`runner.stats().rejected_cases == 0` — a valid-by-construction check on the
generator — separate from coverage gates; it says nothing about whether the
invariant ran.

### 6. Validate the exploration strategy

- Confirm that every intended equivalence class and boundary is in the
  generator support.
- Confirm that all loops, recursive generators, and run-to-quiescence phases
  have explicit bounds.
- Estimate the miss probability of each gate. If a case reaches the target
  with probability `p`, a run of `N` cases misses it entirely with
  probability `(1 - p)^N`. When that number is too high, fix the generator
  in this order — (1) confirm the class is in support, (2) restructure the
  target as a first-class branch (`sample_weighted_index` arm or dedicated
  sampler), (3) assign an explicit weight or `Ratio` via
  `sample_with_boundaries`, (4) only then raise `N`. Raising `N` reduces
  miss probability exponentially only at rate `p`, so a small `p` costs
  many extra cases to compensate.
- Record the `p` estimate and the branch weights it came from next to each
  gate. Every time you change a branch weight, boundary set, choice pool,
  or bounded range, re-check each gate: a gate whose region has become
  unreachable turns green silently, and a gate whose region has become
  saturating adds noise but no coverage.
- Inspect `runner.stats()` when rejection behavior matters.
- Evaluate search changes across several fixed seeds and realistic case
  budgets. Do not judge a distribution from one lucky run.
- When practical, inject or retain a known defect and verify that the property
  detects it reliably. A property that only passes is not evidence that its
  exploration strategy is effective.
- Run the target project's formatting, tests, lints, and doctests in the form
  required by that project.

## Start from this complete test shape

```rust
#[test]
fn vec_matches_model() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("MYAPP_SEED")?;
    let meaningful_pops = std::cell::Cell::new(0usize);
    let mut runner = noprop::Runner::new(seed);

    runner.run(256, |ctx| {
        let mut model = Vec::<u32>::new();
        let mut sut = Vec::<u32>::new();
        let steps = noprop::sample_with_boundaries(
            ctx,
            &[0usize, 1, 32],
            noprop::Ratio::one_nth(5),
            |ctx| noprop::sample_usize_in(ctx, 0..=32),
        );

        for step in 0..steps {
            match noprop::sample_weighted_index(ctx, &[3, 2]) {
                0 => {
                    let value = noprop::sample_u32(ctx);
                    model.push(value);
                    sut.push(value);
                }
                _ => {
                    let expected = model.pop();
                    let actual = sut.pop();
                    assert_eq!(actual, expected, "pop mismatch at step {step}");
                    if expected.is_some() {
                        meaningful_pops.set(meaningful_pops.get() + 1);
                    }
                }
            }
        }
        Ok(())
    })?;

    assert!(
        meaningful_pops.get() > 0,
        "no case exercised a non-empty pop\n{runner}"
    );
    Ok(())
}
```

Replace the model, SUT, commands, weights, bounds, and coverage gate from the
domain analysis. Do not preserve the example's numbers without justification.

## Keep these constraints in mind

- Keep properties and generators as plain Rust closures and functions; do not
  introduce a macro or combinator DSL around noprop.
- In new examples, prefer explicit `noprop::Runner`, `noprop::Ratio`, and
  `noprop::sample_*` paths so the source of each testing primitive stays
  visible. Respect the consuming project's style; do not rewrite existing
  imports solely to enforce this preference.
- Use `sample_usize_in`, not `sample_usize(ctx) % n`.
- Use exact `Ratio` values and validate runtime-derived numerator and
  denominator values before constructing them.
- Require `panic = "unwind"`.
- Expect seed-based reproduction rather than automatic shrinking or on-disk
  failure persistence.

When a failure occurs, rerun the exact entry point, seed, case budget, and
property closure printed by the failure report. Read
[references/failure-diagnostics.md](references/failure-diagnostics.md) only
when reproducing or interpreting an actual failure.
