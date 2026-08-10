---
name: noprop
description: >
  Implement, review, debug, and explain Rust property tests that use the
  noprop crate. Use when a task names noprop, contains `noprop::` APIs or a
  Cargo dependency on noprop, or asks specifically about noprop's Runner,
  TestCaseContext, Ratio, `sample_*`, rejection, failure reproduction, or
  feedback-guided search. Do not activate for generic Rust property-testing
  tasks that use another framework.
---

# noprop

Use noprop to write imperative property tests as plain Rust closures. Focus
first on making the intended bug region reachable and likely to be explored;
API selection and case count come after the search space is sound.

## Follow this workflow

### 1. Establish the applicable API

- Treat the guidance in this skill as targeting noprop 0.1.0.
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

### 4. Choose the search policy

Use `Runner::run` by default. It provides a clear baseline and is the right
choice when direct sampling and explicit bias can reach the important regions.

Use `Runner::run_feedback_guided` only when all of these hold:

- the failure requires semantic progress through a rare region;
- ordinary generator bias would be awkward or insufficient;
- the property can report stable, low-cardinality progress signals; and
- mutating a previously interesting draw sequence is likely to advance the
  property further.

Do not use feedback-guided search to compensate for missing generator support:
it cannot create values the generator cannot produce.

Report semantic feedback as follows:

- Use `ctx.event(label)` for finite milestones or noteworthy occurrences.
- Use `ctx.bucket(label, bucket)` for a state value after mapping it into
  roughly 3-10 meaningful buckets.
- Use `ctx.transition(label, from, to)` for abstract state-machine changes.
- Keep labels static and semantic. Do not report raw timestamps, sequence
  numbers, byte counts, IDs, or other high-cardinality values as features.

Treat feedback as a steering signal, not proof of coverage. Keep assertions
and coverage gates independent of feature reporting.

### 5. Use rejection at the narrowest scope

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

### 6. Prevent vacuous success

Place assertions where the relevant behavior occurs, and separately count
whether that location was reached. After the run, fail if the count is zero.
Use `Cell`, `RefCell`, or atomics for cross-case observations because the
property closure implements `Fn`.

Count only evidence from cases that reach the intended check. Do not treat
attempt count, feedback registration, or rejected cases as proof that the
invariant ran. Do not reject a case after recording coverage evidence; a later
rejection would let a discarded case inflate the gate.

### 7. Validate the exploration strategy

- Confirm that every intended equivalence class and boundary is in the
  generator support.
- Confirm that all loops, recursive generators, and run-to-quiescence phases
  have explicit bounds.
- Inspect `runner.stats()` when rejection or feedback behavior matters.
- Evaluate search changes across several fixed seeds and realistic case
  budgets. Do not judge a distribution or feedback design from one lucky run.
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
