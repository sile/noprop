# RFC: Add generator-sizing and gate-construction guidance to the skill

- Status: accepted

## Summary

Add three pieces of guidance to `skills/noprop/SKILL.md`, each of which a
real noprop consumer arrived at only after a first run went wrong:

1. Estimate a recursive generator's cost as `branch_factor ^ depth` before
   the first run, and keep both numbers as named constants.
2. When a coverage gate needs two independent draws to agree, make one draw
   decide both instead of relying on the two agreeing by chance.
3. When a gate has no single observable signal, compare against an
   alternative strategy per case (an increment), not as a running total.

Nothing else in the skill changes. The proposal is additive prose; no API,
recipe, or code in `src/` or `docs/` is touched.

## Motivation

The skill already states the *rules* these three points sit under —
"keep loops and recursion explicitly bounded", "Estimate the miss
probability of each gate", and "Decide what evidence will prove that the
meaningful region was actually exercised. Add a coverage gate when the
invariant can otherwise pass vacuously". What it does not give is the
step from the rule to a concrete first design, and in three recorded
consumer runs that gap cost a debugging cycle each:

- A recursive generator was first written with `MAX_GEN_DEPTH = 6` and a
  branch factor of 3. The test pinned a core at 100% CPU and timed out;
  the per-case cost was `3^6 = 729` expansions at that bound. The fix was
  `depth = 3`, `branch = 2`. The rule was already read; the *arithmetic*
  was not done, because nothing in the skill says to put a number on it
  first.
- A gate that required a frame's previous and current size to be equal
  reached its branch in roughly 1 case in 169, because the two sizes were
  drawn independently from the 13 distinct sizes the generator produced.
  The generator was valid and the gate was correct; the gate was simply
  flaky. The fix was structural — draw a coin, and have both sizes obey it
  — which removes the coincidence from the design rather than tuning
  weights.
- A gate meaning "the incremental redraw path was taken" had no direct
  signal, so it was expressed as "fewer cells written than a full redraw".
  Compared as a running total, the gate was true on every case, because the
  total always includes the first frame's writes. It only became a real
  gate as a per-case increment.

Each of these is a one-line addition to a section that already exists. The
common shape is that the rule is stated but the *observable failure mode*
and the *first-design default* are not, so a reader applies the rule only
after seeing the failure.

## Guide-level explanation

After this change, a reader who is about to write a recursive generator or
a coverage gate finds the concrete default next to the rule:

- Recursion: pick `depth` and `branch_factor` deliberately, multiply them
  out, and write the product down. If `branch_factor ^ depth` is in the
  thousands, the first run will be slow no matter how cheap one node is.
  Start from `depth = 3`, `branch = 2` and raise a bound only with a reason.
- Gates: if reaching the gated code depends on two or more draws agreeing,
  restructure so that one draw determines all of them, then the gate's
  reachability is a property of the design instead of a probability.
- Gates without a direct signal: if the gate is "strategy B was used rather
  than strategy A", measure a per-case increment of the observable both
  strategies move. A cumulative total will already be non-zero from earlier
  cases and proves nothing.

Nothing here changes how a property is written; it changes what the reader
checks before pressing run.

## Reference-level explanation

Three edits to `skills/noprop/SKILL.md`, all prose, no new sections.

1. In section 3 (Design an effective search space), extend the existing
   bullet "Keep loops and recursion explicitly bounded" with the cost
   estimate and a starting point:

   - State that a recursive generator's per-case work grows as
     `branch_factor ^ depth`, where `depth` is the nesting count and
     `branch_factor` the number of children per level, so both must be
     chosen together and kept as named constants in one place.
   - Give a default to start from (`depth` in the low single digits, a
     branch factor of 2) and say to relax a bound only with evidence that
     more is needed.

2. In section 5 (Prevent vacuous success), extend the guidance on choosing
   a gate with a bullet on correlated gates:

   - When the gated path is reachable only if two or more draws agree
     (equal sizes, equal lengths, a match between two independent samples),
     derive all of them from one draw rather than drawing them
     independently.
   - Name the failure mode: independent draws make the gate's reachability
     a product of probabilities that is usually far below the weight a
     reader imagines, and the gate fails intermittently rather than never.

3. In section 5, add a companion bullet on gates with no direct signal:

   - When the gate means "alternative strategy B was taken" and there is no
     flag for B, compare the observable both strategies move, but compare a
     *per-case increment*, not a cumulative total.
   - State why: a cumulative total includes the work of every earlier case,
     so it is positive from the first case and the gate is vacuous.

Section 6 (Validate the exploration strategy) already tells the reader to
estimate a gate's miss probability; edits 2 and 3 give the design move that
makes the estimate moot, so no change is needed there beyond the existing
text.

## Drawbacks

- **The skill gets longer.** `SKILL.md` is read into a model's context, so
  every added line competes with the rest. The three additions are a few
  lines each and attach to existing bullets, but the total does grow.
- **Numbers in prose go stale.** `depth = 3` / `branch = 2` is a starting
  point from one family of generators, not a law. A reader who treats it as
  a target could under-generate in a domain where a larger bound is
  correct. The text must say "start from", not "use".
- **Overlap with section 6.** The gate bullets restate, in design terms,
  what section 6 already says in probability terms. A careful reader may
  find the second statement redundant; the justification is that the first
  is read before the run and the second after a failure.

## Rationale and alternatives

- **Do nothing.** The rules are already present, and each consumer
  eventually fixed its own case. The cost is that the fix came from a
  failed run rather than from reading, three times, in independent projects.
  A skill exists to move that cost left; leaving it out keeps the skill
  correct but not yet useful at the moment of first design.
- **Add the guidance to `docs/recipes.md` instead.** The recipes file is
  for consumers writing properties in their own repository and is much
  larger; a reader there is looking for a worked example, not for a default
  to apply while designing. The three points are defaults, which is what
  the skill is for.
- **Add the guidance to the README instead.** The README is deliberately
  kept to a quick start and a policy summary, with details in the skill,
  `docs/`, or rustdoc; a generator-sizing heuristic is not a policy the
  README's reader needs at first read. Putting it there would also widen
  the surface the README's summary has to stay consistent with.
- **Add a fourth example to the "Start from this complete test shape"
  block.** The block demonstrates a single model/SUT loop; recursion and
  gate construction are orthogonal to it, and folding them in would make
  the one canonical example harder to lift. Prose attached to the existing
  bullets is the smaller change.
- **Give exact node budgets.** Tempting, but the per-node cost is domain
  dependent and any number would be wrong somewhere. The estimate
  (`branch_factor ^ depth`) and the starting point carry the same decision
  without pretending to a hard limit.
- **Merge the two gate bullets into one.** They address different mistakes
  (gate too unlikely vs. gate always true) and a reader hits one or the
  other, not both. Keeping them separate lets each be found by its symptom.

## Outcome

Accepted. The three additions go into `skills/noprop/SKILL.md` as described
in [Reference-level explanation](#reference-level-explanation). Both open
questions were settled against adding more:

- **No total-nodes budget.** The per-case cost (`branch_factor ^ depth`) is
the quantity the reader is missing; the total over a run is that cost times
the case count, and the case count is already governed by section 6's miss
probability guidance. A second budget would give the same advice twice while
suggesting a measurement the skill does not ask for, and an exact node count
is domain dependent enough that this proposal already rejected it as an
alternative. The estimate alone is what makes the reader pick small numbers;
the multiplication is the whole step.
- **No inline worked example for the correlated gate.** The bullet stays at
the level of "derive them from one draw". Section 5 is dense and its other
bullets carry decisions, not examples; an equal-size example needs domain
vocabulary (frames, sizes) that the rest of the section avoids. The failure
mode is already named (independent draws make reachability a product of
probabilities, so the gate fails intermittently), which is what a reader
needs to recognize and fix their own case. The arithmetic stays in
Motivation, where it justifies the rule rather than restating it.

## Unresolved questions

None. Both questions raised in draft were settled in
[Outcome](#outcome).

## Future possibilities

- If more consumer runs surface the same three failure modes, the bullets
  could be promoted to a short checklist at the top of section 5.
- The same "state the rule, then state the first-design default" treatment
  may be worth applying to the rejection and boundary sections, which
  currently have rules but no defaults; that is out of scope here.
