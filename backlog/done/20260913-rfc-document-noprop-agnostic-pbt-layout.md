# RFC: Document noprop-agnostic pbt layout

- Status: rejected

## Summary

Add a short note to noprop's own material stating what noprop does and does
not require of a property-testing layout. The note would say that noprop is a
dependency-free library meant for test code, so where the tests live is a
project decision that noprop does not constrain.

This proposal was **rejected**. The reasoning is kept here so the request is
not raised again without new evidence. The rejection and its grounds are in
[Outcome](#outcome).

## Motivation

noprop is consumed by projects that put their property tests in several
different places: a top-level `pbt/` subcrate, a flat `tests/pbt_harness.rs`
module, a `#[cfg(test)]` module inside `src/`, or a single `tests/pbt.rs`.
Projects that grow past a handful of properties end up asking the same
question -- where should the tests live, and does noprop care? -- and the
answer is not evident from noprop's own documentation.

The request that prompted this proposal was to document the subcrate
arrangement as a recipe: when to choose a `pbt/` subcrate, a minimal
`Cargo.toml` for it, and how to keep it in CI. A survey of ten consumers found
four comparable projects that had converged on top-level `pbt/` subcrates, so
the request was not idle.

The narrower question this proposal answers is whether noprop should say
anything about layout at all, and if so, how much.

## Guide-level explanation

If this proposal is accepted, a user who asks "does noprop work in a `pbt/`
subcrate?" gets an answer from noprop itself: yes, and so does every other
arrangement, because noprop is a library with no build-time or runtime
requirements beyond the standard library.

What the user would *not* get is guidance on choosing between arrangements.
The proposal deliberately stops at stating noprop's own properties. It does
not say when a subcrate is worth the indirection, does not show a
`Cargo.toml`, and does not describe a CI job.

A user who asks "should I put my property tests in a `pbt/` subcrate?" still
has to answer that from their own project's constraints.

## Reference-level explanation

The change is one or two sentences in noprop's own material -- the README, or
wherever noprop describes what it is. The substance is three statements, each
of which is a fact about noprop rather than about layout:

- noprop is intended for test code. It can be a `[dev-dependencies]` entry of
the crate under test, or a `[dependencies]` entry of a separate test-only
crate. Which one applies follows from Cargo's target rules, not from noprop.
- noprop has no dependencies of its own. It therefore never adds build cost to
the crate it is placed in. This removes one of the usual reasons to isolate
property tests into a separate crate.
- noprop's randomness comes from an explicit seed and nothing else, so
reproducing a failure works the same whether the tests live in the main crate
or in a subcrate with its own CI job. Splitting the tests out cannot break
reproducibility in a way noprop would need to warn about.

No code changes. No new file under `docs/`.

## Drawbacks

Even the short note is a maintenance obligation. It is a claim about how
noprop interacts with Cargo, and Cargo changes. A statement that is true today
-- "noprop has no dependencies, so it adds no build cost" -- would become
misleading if noprop ever gained a dependency, and someone would have to
notice and update it.

The note also does not fully answer the original request. A maintainer
starting a property-testing effort still has to work out the layout on their
own, and the knowledge that four comparable projects converged on top-level
`pbt/` subcrates stays with those projects.

## Rationale and alternatives

Alternatives considered:

- **Document the subcrate layout as a recipe.** Rejected. The guidance would
  be substantially the same for any property-testing library, so it is not
  noprop's knowledge to publish, and it would be a maintenance obligation tied
  to Cargo rather than to noprop.
- **Add the short note only, not a recipe (this proposal).** Rejected, and
  this is the closest call. The note is accurate, useful, and genuinely about
  noprop. It is nevertheless declined for the reason in [Outcome](#outcome):
  every fact in it is one a reader can derive from noprop's existing
  description in seconds, while the sentence itself has to be maintained.
- **Document the layout in the README instead of `docs/`.** Rejected for the
  same reason as the recipe, with the added problem that the README is the
  first thing a new user reads and should describe noprop, not Cargo.
- **Do nothing and leave the request open.** Accepted for the decision itself
  (the request is declined), but the RFC is still written, because an open
  request with no record keeps the question alive.

The impact of doing nothing is the same as the impact of this proposal: no
noprop material changes.

## Outcome

**Rejected.** Three observations decided it.

First, the reasons projects split out a `pbt/` subcrate are all project
specific. In the repositories surveyed, the motivations were: keeping a heavy
`[dev-dependencies]` set out of the main crate's ordinary test build; testing
`pub(crate)` items that an integration test cannot reach; and giving property
tests their own CI job so the main job stays fast. None of these is caused by
noprop, and none changes when noprop is swapped for another property-testing
library.

Second, a document users read has to stay true as both noprop and the Rust
toolchain evolve. Layout guidance ages with Cargo, not with noprop. Even a
short note creates a claim that noprop's own releases never touch.

Third, the survey turned up no arrangement that noprop rules out. If a layout
were incompatible with noprop -- for example, if a subcrate could not reach
the seed helpers, or if test-only placement changed behavior -- that would be
a reason to write it down, because the constraint would come from noprop and
would need to be explained. No such constraint was found.

The short note from the previous section is declined on the same grounds: it
restates what a reader can already see from noprop's description (it has no
dependencies; it is for tests), so the maintenance cost buys no new
information.

## Unresolved questions

None. The decision is a scope boundary, and the boundary is stated in
[Outcome](#outcome).

## Future possibilities

If noprop ever gains a component that does constrain layout -- a proc-macro
that must be declared in a particular crate, a runtime that installs
per-process state, or anything else with a placement requirement -- that
constraint would belong in noprop's documentation, and this decision would not
apply to it. The reasoning here is specific to a dependency-free, macro-free
library whose tests can live anywhere.

A separate proposal covering what noprop should say about itself in the README
is possible. If it is written, it should not restate the rejected note here;
the rejection reason applies to any noprop self-description whose content is
derivable from the existing one.
