# Generator Authoring

Reference for writing `sample_*` helpers with noprop. Every generator
has the shape
`fn sample_X(ctx: &mut TestCaseContext) -> X`, so user
generators (`fn sample_person(ctx: &mut TestCaseContext) -> Person`)
compose with the built-in ones by plain function call. This document
gathers the composition patterns, the shared rejection-sampler
contract, the `NonZero<_>` recipes, and the finite-by-default float
samplers.

For the underlying design decisions (support, distribution,
termination, rejection scope, valid-by-construction) see
[`crate::docs::generator_design`].

## Composing generators

To build a generator whose output depends on another's, just call
them sequentially inside a plain function or closure:

```rust
use noprop::TestCaseContext;

fn sample_bounded_vec(ctx: &mut TestCaseContext) -> Vec<u32> {
    // Pick a length first, then a Vec of that length.
    let len = noprop::sample_usize_in(ctx, 0..10);
    (0..len).map(|_| noprop::sample_u32(ctx)).collect()
}

let mut ctx = TestCaseContext::new(0);
let _v: Vec<u32> = sample_bounded_vec(&mut ctx);
```

For "one-of-N" branching between code paths, `match` on a small
random value produced by [`sample_usize_in`](crate::sample_usize_in):

```rust
let mut ctx = noprop::TestCaseContext::new(0);
let _x: u32 = match noprop::sample_usize_in(&mut ctx, 0..3) {
    0 => 0,
    1 => noprop::sample_u32(&mut ctx),
    _ => u32::MAX,
};
```

To weight the branches unevenly, use
[`sample_weighted_index`](crate::sample_weighted_index) (or
[`sample_ratio`](crate::sample_ratio) for a two-way split):

```rust
let mut ctx = noprop::TestCaseContext::new(0);
// Pick branch 0 with weight 5, branch 1 with weight 3, branch 2 with weight 2.
let _x: u32 = match noprop::sample_weighted_index(&mut ctx, &[5, 3, 2]) {
    0 => 0,
    1 => noprop::sample_u32(&mut ctx),
    _ => u32::MAX,
};
```

To pick one value from a fixed list, use
[`sample_choice`](crate::sample_choice):

```rust
let mut ctx = noprop::TestCaseContext::new(0);
let _n = noprop::sample_choice(&mut ctx, &[1, 2, 3, 5, 8]);
let _digit = noprop::sample_choice(&mut ctx, b"0123456789") as char;
```

For bounded retry (filter-style generation), combine a range iterator
with `.find()`:

```rust
let mut ctx = noprop::TestCaseContext::new(0);
let even: Option<u32> = (0..100)
    .map(|_| noprop::sample_u32(&mut ctx))
    .find(|x| x % 2 == 0);
# assert!(even.is_some());
```

## Bounded rejection sampling

When a generator has to keep drawing until it hits a value that
satisfies some predicate, use
[`sample_with_rejection`](crate::sample_with_rejection) rather than
a hand-written `loop { … }`. Unbounded manual retries can wedge on
specific choice sequences; the helper enforces a `max_attempts`
bound and, on exhaustion, calls
[`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case)
so the enclosing [`Runner::run`](crate::Runner::run) discards the
iteration and moves on. Prefer valid-by-construction generators when
the accepted rate is very low.

## Sampling non-zero integers

noprop deliberately does not ship dedicated `sample_non_zero_*`
primitives. The `NonZero<_>` domain is not one shape — callers make
a real trade-off between distribution uniformity and unconditional
termination — so `NonZero` is left as a two-line recipe over the
plain integer sampler. Pick one of the two:

**Uniform, may reject the iteration.** Use
[`sample_with_rejection`](crate::sample_with_rejection) to redraw
until the sampled integer is non-zero. `P(zero)` is at most `1/256`
per attempt (worst case, `u8`), so the shared 64-attempt bound is
effectively unreachable — but on exhaustion the iteration is
rejected, so this recipe requires a [`Runner`](crate::Runner) around
it.

```rust
# let _: noprop::RunResult = noprop::Runner::new(0).run(1, |ctx| {
use std::num::NonZeroU32;
let n = noprop::sample_with_rejection(ctx, 64, |ctx| {
    NonZeroU32::new(noprop::sample_u32(ctx))
});
assert!(n.get() != 0);
# Ok(())
# });
```

**Biased, always terminates in one draw.** Map the underlying
integer's zero value onto `1` explicitly. This shifts a small
amount of probability mass onto `1` (worst case `+1/256` for `u8`)
but avoids any retry loop and works outside a `Runner`.

```rust
use std::num::NonZeroU32;
let mut ctx = noprop::TestCaseContext::new(0);
let v = noprop::sample_u32(&mut ctx);
let n = NonZeroU32::new(if v == 0 { 1 } else { v })
    .expect("v was remapped away from zero");
assert!(n.get() != 0);
```

`wrapping_add(1)` is *not* a correct substitute — it wraps
`u_::MAX` back to `0`, reintroducing the very case the mapping is
meant to eliminate. `saturating_add(1)` avoids `0` on unsigned
types but overweights the maximum, and does not work on signed
types at all. The explicit `if v == 0 { 1 } else { v }` shows the
chosen remapping target in the code.

For signed types, note that the full `NonZero<i_>` domain is
`MIN..=-1` ∪ `1..=MAX`. A single `1..=MAX` range would silently
drop the negative half, so the uniform recipe (rejection sampling
over [`sample_i32`](crate::sample_i32), etc.) is the only way to
cover both signs uniformly.

Note that neither recipe records the resulting `NonZero<_>` as its
own trace entry — the underlying integer sample is what appears in
the failure trace. Wrap the value manually with
[`sample_choice`](crate::sample_choice) or a custom
`#[track_caller]` helper if you want a `NonZero`-typed trace entry
at the call site.

## Sampling floats

[`sample_f32`](crate::sample_f32) and
[`sample_f64`](crate::sample_f64) return finite values by default
(excluding `NaN` and `±∞`) via a small bounded rejection loop, so
callers do not need to write their own `is_finite` filter around the
integer primitives. For a uniform draw over a bounded range, use
[`sample_f32_in`](crate::sample_f32_in) /
[`sample_f64_in`](crate::sample_f64_in). To sample an arbitrary bit
pattern (including `NaN`, infinities, and subnormals), build it
explicitly from [`sample_u32`](crate::sample_u32) /
[`sample_u64`](crate::sample_u64), e.g.
`f32::from_bits(noprop::sample_u32(ctx))`.
