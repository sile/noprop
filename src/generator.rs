//! Value generators. See [`crate::docs::generator_authoring`] for the
//! authoring guide (composition patterns, bounded rejection, `NonZero`
//! recipes, floats).

use std::ops::{Bound, RangeBounds};
use std::panic::Location;

use crate::TestCaseContext;
use crate::rng::{AttemptVerdict, ChoiceMeta};

/// Read `N` bytes from `ctx` without recording. Used by every primitive
/// so that composite generators (non-zero variants, `sample_char`,
/// floats, `sample_choice`) can consume randomness without producing
/// intermediate trace entries for the raw byte source.
fn raw_bytes<const N: usize>(ctx: &mut TestCaseContext) -> [u8; N] {
    let mut buf = [0u8; N];
    ctx.fill(&mut buf);
    buf
}

// === Bounded rejection sampling ===

/// Shared attempt limit for every internal bounded rejection loop in
/// this module. Chosen so that a per-attempt rejection rate up to 50%
/// (the worst case for [`sample_below`]) still exhausts with
/// probability `< 2⁻⁶⁴`.
pub(crate) const DEFAULT_MAX_ATTEMPTS: usize = 64;

/// Repeatedly invoke `attempt` up to `max_attempts` times until it
/// returns `Some`, then return that value. On exhaustion (all attempts
/// returned `None`) this calls
/// [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case), which unwinds out to
/// the enclosing [`Runner::run`](crate::Runner::run) and marks the
/// current iteration as rejected — so this function's return type is
/// `T`, not `Option<T>`.
///
/// The helper is the noprop-endorsed way to write bounded rejection
/// sampling. Prefer it over hand-written `loop { … }` retries: those
/// can spin forever on choice sequences where every draw fails the
/// predicate, and cannot signal iteration-level rejection to the
/// runner. Use `reject_case` directly only when the whole iteration is
/// unsuitable after sampling has already finished.
///
/// # Behavior
///
/// - The first `Some(value)` returned by `attempt` is passed through
///   unchanged; subsequent attempts are not tried.
/// - `None` is treated as a rejected attempt and the next attempt is
///   tried.
/// - `max_attempts` counts the total attempts including the first, so
///   `max_attempts == 1` gives one shot; `max_attempts == 0` is a
///   programmer error and panics.
/// - An attempt that consumes zero draws (a drawless filter, e.g. a
///   pure predicate over external state) is allowed. In recording
///   mode its span is stored with `start_draw == end_draw`.
/// - The closure may itself call `sample_with_rejection` — nested
///   attempts are supported and recorded with parent/child linkage in
///   recording mode.
/// - A user panic inside the closure (other than the private
///   iteration-rejection marker sent by `reject_case`) propagates
///   verbatim; the runner's own `catch_unwind` handles it as a
///   property failure.
///
/// # Determinism note
///
/// This is a control-flow boundary, not a value primitive. The
/// accepted `T` is not recorded as a new `GeneratedValue` entry — the
/// primitives called inside `attempt` are still recorded normally, so
/// the trace shows the actual draws rather than a redundant wrapper.
///
/// # Panics
///
/// Panics if `max_attempts == 0`.
///
/// # Examples
///
/// ```
/// # let _: noprop::RunResult = noprop::Runner::new(0).run(1, |ctx| {
/// // Sample an even u32 in at most 8 attempts. If all 8 attempts are
/// // odd (probability 1/256), the iteration is rejected and Runner
/// // tries the next one.
/// let even = noprop::sample_with_rejection(ctx, 8, |ctx| {
///     let x = noprop::sample_u32(ctx);
///     if x % 2 == 0 { Some(x) } else { None }
/// });
/// assert_eq!(even % 2, 0);
/// # Ok(())
/// # });
/// ```
#[track_caller]
pub fn sample_with_rejection<T, F>(
    ctx: &mut TestCaseContext,
    max_attempts: usize,
    mut attempt: F,
) -> T
where
    F: FnMut(&mut TestCaseContext) -> Option<T>,
{
    assert!(
        max_attempts > 0,
        "sample_with_rejection: max_attempts must be > 0"
    );
    for _ in 0..max_attempts {
        let id = ctx.begin_attempt();
        match attempt(ctx) {
            Some(value) => {
                ctx.end_attempt(id, AttemptVerdict::Accepted);
                return value;
            }
            None => {
                ctx.end_attempt(id, AttemptVerdict::Rejected);
            }
        }
    }
    ctx.reject_case()
}

// === Bounded-domain sampler ===

/// Sample a uniform `u64` in `[0, n)` using rejection sampling.
///
/// Uses `u64` as a pointer-width-independent working domain so the same
/// draw pattern applies to every finite-domain selection primitive
/// (`sample_usize_in`, `sample_ratio`, `sample_weighted_index`,
/// `sample_choice`).
/// Draws are consumed from the RNG only via [`raw_bytes`], so rejected
/// attempts do not appear in the value trace (rejection span metadata
/// is still recorded in Recording mode).
///
/// Panics in debug builds if `n == 0`. Callers must guarantee `n > 0`.
///
/// The internal rejection loop is bounded at `DEFAULT_MAX_ATTEMPTS`
/// via [`sample_with_rejection`]. Per-attempt rejection rate is at
/// most `(u64::MAX % n + 1) / 2⁶⁴`, which peaks near 50% when `n` is
/// just above a power of two; the probability of exhausting 64
/// attempts is therefore `< 2⁻⁶⁴`.
#[track_caller]
fn sample_below(ctx: &mut TestCaseContext, n: u64) -> u64 {
    sample_below_with_meta(ctx, n, ChoiceMeta::Bounded { bound: n })
}

/// `sample_below` with an explicit [`ChoiceMeta`] for the draws it
/// consumes. `sample_choice` uses this to tag its index draw as a
/// `Choice` instead of a plain `Bounded` draw, so the metadata records
/// the primitive that produced it rather than the shared core.
#[track_caller]
fn sample_below_with_meta(ctx: &mut TestCaseContext, n: u64, meta: ChoiceMeta) -> u64 {
    debug_assert!(n > 0, "sample_below: n must be non-zero");
    if n == 1 {
        return 0;
    }
    // We want to sample uniformly from [0, n) by drawing x from
    // [0, 2^64) and returning x % n. To avoid modulo bias we must only
    // accept draws that fall in a range whose size is a multiple of n.
    //
    // Let r = u64::MAX % n. Then 2^64 mod n = (r + 1) mod n:
    //   - If r == n - 1, n divides 2^64: every draw is unbiased.
    //   - Otherwise the unbiased zone has 2^64 - (r + 1) = u64::MAX - r
    //     values, i.e. accept iff x < u64::MAX - r.
    let r = u64::MAX % n;
    if r == n - 1 {
        ctx.set_next_choice_meta(meta);
        return u64::from_le_bytes(raw_bytes(ctx)) % n;
    }
    let bound = u64::MAX - r;
    sample_with_rejection(ctx, DEFAULT_MAX_ATTEMPTS, |ctx| {
        ctx.set_next_choice_meta(meta);
        let x = u64::from_le_bytes(raw_bytes(ctx));
        (x < bound).then_some(x % n)
    })
}

// === Selection helpers ===

/// Pick one element from `choices` uniformly at random.
///
/// This is the noprop counterpart to picking from a fixed list. Use it
/// when the alternatives are *values*; for branching between code paths
/// (calling different generators, taking different actions), use `match`
/// on [`sample_usize_in`] or [`sample_weighted_index`] instead — see the
/// module docstring.
///
/// # Panics
///
/// Panics if `choices` is empty.
///
/// # Determinism note
///
/// Uses the same rejection-sampling core as [`sample_usize_in`]. For
/// most slice lengths the rejection rate is negligible, but the number
/// of RNG bytes consumed by a call is not fixed, so the exact output
/// stream for a given seed depends on the slice length.
///
/// # Examples
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0);
/// // Explicit list of ints
/// let _n = noprop::sample_choice(&mut ctx, &[1, 2, 3, 5, 8]);
/// // ASCII digit from a byte string literal
/// let _d = noprop::sample_choice(&mut ctx, b"0123456789") as char;
/// // Non-ASCII via array literal
/// let _c = noprop::sample_choice(&mut ctx, &['α', 'β', 'γ']);
/// ```
#[track_caller]
pub fn sample_choice<T: Clone + std::fmt::Debug + 'static>(
    ctx: &mut TestCaseContext,
    choices: &[T],
) -> T {
    assert!(!choices.is_empty(), "sample_choice: empty slice");
    let loc = Location::caller();
    let idx = sample_below_with_meta(
        ctx,
        choices.len() as u64,
        ChoiceMeta::Choice { len: choices.len() },
    ) as usize;
    let v = choices[idx].clone();
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `usize` inside `range`.
///
/// Accepts any `RangeBounds<usize>` — `a..b`, `a..=b`, `..b`, `a..`,
/// `..`, and so on. The typical use is picking a collection length,
/// slice index, loop count, or small branch discriminator without
/// having to write `% N` (which is both easy to overflow at
/// `usize::MAX` and biased when the divisor does not evenly divide the
/// integer domain).
///
/// # Panics
///
/// Panics if `range` is empty (e.g. `5..5`, `5..=4`, `..0`, or an
/// excluded start of `usize::MAX`).
///
/// # Determinism note
///
/// Uses rejection sampling internally. Most calls accept the first
/// draw, but the exact byte count consumed depends on the range width,
/// so changing the range can shift subsequent RNG output for the same
/// seed. This is a deliberate correction over the earlier `sample_usize
/// % (max + 1)` recipe, which was both bias-prone and overflow-prone.
///
/// # Examples
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0);
///
/// let idx = noprop::sample_usize_in(&mut ctx, 0..10);
/// assert!(idx < 10);
///
/// let day = noprop::sample_usize_in(&mut ctx, 1..=31);
/// assert!((1..=31).contains(&day));
/// ```
#[track_caller]
pub fn sample_usize_in<R: RangeBounds<usize>>(ctx: &mut TestCaseContext, range: R) -> usize {
    let loc = Location::caller();
    let lo = match range.start_bound() {
        Bound::Included(&s) => s,
        Bound::Excluded(&s) => s.checked_add(1).expect("sample_usize_in: empty range"),
        Bound::Unbounded => 0,
    };
    let hi = match range.end_bound() {
        Bound::Included(&e) => e,
        Bound::Excluded(&e) => e.checked_sub(1).expect("sample_usize_in: empty range"),
        Bound::Unbounded => usize::MAX,
    };
    assert!(lo <= hi, "sample_usize_in: empty range");
    let v = if lo == 0 && hi == usize::MAX {
        // Full pointer-width range: a raw byte draw is already unbiased,
        // and hi - lo + 1 would wrap. This is a plain integer draw.
        ctx.set_next_choice_meta(crate::rng::ChoiceMeta::Integer);
        usize::from_le_bytes(raw_bytes(ctx))
    } else {
        // hi - lo cannot overflow because hi >= lo, and (hi - lo) + 1
        // cannot overflow because we excluded the only case where
        // hi - lo == usize::MAX. Cast to u64 is safe on every Rust
        // target (usize width <= 64).
        let width = (hi - lo) as u64 + 1;
        lo + sample_below(ctx, width) as usize
    };
    ctx.record_generated(&v, loc);
    v
}

/// An exact rational probability `numerator / denominator`.
///
/// The ratio is valid by construction: `denominator` is non-zero and
/// `numerator <= denominator`, so the probability always lies in
/// `[0, 1]`.
///
/// A ratio is a probability, not a fraction of other quantities:
/// `Ratio::new(1, 3)` means exactly one-in-three — the sampling core
/// compares against `denominator` directly, so the value stays exact
/// rather than a `0.333…`-close float.
///
/// # Choosing the constructor
///
/// The two entry points map to the two shapes real callers hit:
///
/// - [`Ratio::one_nth(n)`](Ratio::one_nth) for the common `1/N`
///   probability. One argument, so no numerator/denominator confusion.
/// - [`Ratio::new(m, n)`](Ratio::new) for a general `m/n` with `m > 1`,
///   as a compile-time literal (numerator-first, matching the
///   mathematical convention).
///
/// Both panic on invalid inputs with a `#[track_caller]` message. That
/// suits compile-time literals — a bad literal is a caller bug that
/// should surface loudly, not a value to branch on. For a runtime value
/// that may be out of range, clamp or validate it yourself before
/// calling `Ratio::new`, for example:
///
/// ```
/// let (n, d) = (3u32, 2u32); // could be anything at runtime
/// let (n, d) = match (n, d) {
///     (0, 0) => (0, 1),
///     (_, 0) => (1, 1),
///     (n, d) if n > d => (d, d),
///     pair => pair,
/// };
/// let r = noprop::Ratio::new(n, d);
/// # let _ = r;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ratio {
    numerator: u32,
    denominator: u32,
}

impl Ratio {
    /// Construct a ratio `numerator / denominator`.
    ///
    /// Intended for compile-time literals where the value is known to
    /// be in range. For `1/N` prefer [`Ratio::one_nth`], which takes a
    /// single argument.
    ///
    /// # Panics
    ///
    /// Panics with a `#[track_caller]` message when the inputs do not
    /// describe a probability:
    ///
    /// - `denominator == 0`
    /// - `numerator > denominator`
    ///
    /// # Examples
    ///
    /// ```
    /// let r = noprop::Ratio::new(2, 3);
    /// # let _ = r;
    /// ```
    #[track_caller]
    pub const fn new(numerator: u32, denominator: u32) -> Self {
        if denominator == 0 {
            panic!("Ratio::new: denominator must be non-zero");
        }
        if numerator > denominator {
            panic!("Ratio::new: numerator must not exceed denominator");
        }
        Self {
            numerator,
            denominator,
        }
    }

    /// Construct the `1/n` ratio.
    ///
    /// The one-argument shortcut for the `1/N` probabilities that
    /// dominate real usage (`Ratio::one_nth(10)` for 1-in-10,
    /// `Ratio::one_nth(2)` for a coin flip).
    ///
    /// # Panics
    ///
    /// Panics with a `#[track_caller]` message when `n == 0`.
    ///
    /// # Examples
    ///
    /// ```
    /// let r = noprop::Ratio::one_nth(10); // 10%
    /// # let _ = r;
    /// ```
    #[track_caller]
    pub const fn one_nth(n: u32) -> Self {
        if n == 0 {
            panic!("Ratio::one_nth: n must be non-zero");
        }
        Self {
            numerator: 1,
            denominator: n,
        }
    }
}

/// Return `true` with probability `ratio`.
///
/// The typical use is weighting a two-way branch by an exact rational
/// probability instead of a floating-point one, so that
/// e.g. `sample_ratio(ctx, Ratio::one_nth(3))` is exactly one-in-three,
/// not `0.333…`-close.
///
/// # Determinism note
///
/// The degenerate cases `numerator == 0` (always `false`) and
/// `numerator == denominator` (always `true`) consume no RNG bytes, so
/// tuning a weight down to 0 or up to 100% does not shift subsequent
/// output. All other cases consume RNG bytes through the shared
/// rejection sampler, and the exact byte count depends on
/// `denominator`.
///
/// # Examples
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0);
/// // 1 in 3 chance of true.
/// let _b = noprop::sample_ratio(&mut ctx, noprop::Ratio::one_nth(3));
/// // Always false; consumes no RNG.
/// assert!(!noprop::sample_ratio(&mut ctx, noprop::Ratio::new(0, 5)));
/// // Always true; consumes no RNG.
/// assert!(noprop::sample_ratio(&mut ctx, noprop::Ratio::new(5, 5)));
/// ```
#[track_caller]
pub fn sample_ratio(ctx: &mut TestCaseContext, ratio: Ratio) -> bool {
    let loc = Location::caller();
    let v = if ratio.numerator == 0 {
        false
    } else if ratio.numerator == ratio.denominator {
        true
    } else {
        sample_below(ctx, ratio.denominator as u64) < ratio.numerator as u64
    };
    ctx.record_generated(&v, loc);
    v
}

/// Return a value from `boundaries` with probability `ratio`, otherwise
/// draw from `sample`.
///
/// This packages the boundary-mixing recipe — an exact-ratio two-way
/// branch between a fixed candidate list and a base generator — into
/// one call. The distribution is readable from the arguments: with
/// probability `ratio` the value is drawn uniformly from `boundaries`,
/// and otherwise it is whatever `sample` produces. The base generator
/// stays untouched, so a `sample_u32`-based property continues to draw
/// uniformly except for the explicitly requested boundary mass.
///
/// `boundaries` may carry domain-level values that an automatic
/// type-level bias could not reach (e.g. an MP4 size field crossing
/// `u32::MAX`, an MTU, a page size).
///
/// This call is a convenience wrapper for the equivalent plain-Rust
/// form
///
/// ```text
/// if sample_ratio(ctx, ratio) {
///     sample_choice(ctx, boundaries)
/// } else {
///     sample(ctx)
/// }
/// ```
///
/// — provided so the boundary-mix recipe fits in one call. The two
/// forms consume the same bytes for the same seed (see the example).
///
/// # Panics
///
/// Panics if `boundaries` is empty, before any RNG bytes are drawn.
///
/// # Determinism note
///
/// The draw order is fixed: the ratio branch first, then either the
/// boundary choice or the `sample` call. A degenerate ratio (0% or
/// 100%) and a one-element boundary slice consume no RNG bytes, so a
/// hand-written recipe built on `sample_usize_in` can be replaced with
/// this helper without shifting the choice sequence. (A `sample_bool`-based
/// recipe is not byte-equivalent: the ratio draws through the shared
/// rejection sampler.)
///
/// The call records two trace entries — the ratio's `bool` and the
/// chosen value — both attributed to the call site; a hand-written
/// recipe records only the value.
///
/// # Examples
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0);
/// // 10% of the time a boundary value, otherwise uniform.
/// let helper = noprop::sample_with_boundaries(
///     &mut ctx,
///     &[0, 1500, u32::MAX],
///     noprop::Ratio::one_nth(10),
///     noprop::sample_u32,
/// );
///
/// // The helper is exactly the plain-Rust `if` form below: the same
/// // seed draws the same bytes, so the values agree.
/// let mut hand_ctx = noprop::TestCaseContext::new(0);
/// let hand_written =
///     if noprop::sample_ratio(&mut hand_ctx, noprop::Ratio::one_nth(10)) {
///         noprop::sample_choice(&mut hand_ctx, &[0, 1500, u32::MAX])
///     } else {
///         noprop::sample_u32(&mut hand_ctx)
///     };
/// assert_eq!(helper, hand_written);
/// ```
#[track_caller]
pub fn sample_with_boundaries<T, F>(
    ctx: &mut TestCaseContext,
    boundaries: &[T],
    ratio: Ratio,
    sample: F,
) -> T
where
    T: Clone + std::fmt::Debug + 'static,
    F: FnOnce(&mut TestCaseContext) -> T,
{
    assert!(
        !boundaries.is_empty(),
        "sample_with_boundaries: empty boundaries"
    );
    if sample_ratio(ctx, ratio) {
        sample_choice(ctx, boundaries)
    } else {
        sample(ctx)
    }
}

/// Pick an index from `weights` with probability proportional to each
/// weight.
///
/// The typical use is picking a branch or command variant with an
/// uneven distribution (e.g. 10× more `read` operations than `write`).
/// The chosen index is returned so the caller can `match` on it and
/// call the corresponding generator or action — the weight list carries
/// no values of its own.
///
/// # Panics
///
/// - Panics if `weights` is empty.
/// - Panics if every weight is `0` (no branch is selectable).
/// - Panics if the sum of weights overflows `u64` (in practice this
///   requires a `weights` slice that cannot fit in addressable memory).
///
/// # Determinism note
///
/// Uses the shared rejection sampler on the sum of weights, so the
/// exact number of RNG bytes consumed depends on that sum. Adding a
/// zero-weighted variant to the slice does change the sampler input
/// (the slice length grows) even though it never affects the returned
/// index distribution.
///
/// # Examples
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0);
/// // Roughly 50% branch 0, 30% branch 1, 20% branch 2.
/// let idx = noprop::sample_weighted_index(&mut ctx, &[5, 3, 2]);
/// assert!(idx < 3);
/// ```
#[track_caller]
pub fn sample_weighted_index(ctx: &mut TestCaseContext, weights: &[u32]) -> usize {
    let loc = Location::caller();
    assert!(!weights.is_empty(), "sample_weighted_index: empty weights");
    let mut total: u64 = 0;
    for &w in weights {
        total = total
            .checked_add(w as u64)
            .expect("sample_weighted_index: weight sum overflows u64");
    }
    assert!(total > 0, "sample_weighted_index: all weights are zero");
    let mut pick = sample_below(ctx, total);
    let mut chosen = weights.len(); // sentinel; overwritten below
    for (i, &w) in weights.iter().enumerate() {
        let w = w as u64;
        if pick < w {
            chosen = i;
            break;
        }
        pick -= w;
    }
    // Every non-zero-weight index is reachable and pick < total, so the
    // loop must have hit the `pick < w` branch at least once.
    debug_assert!(chosen < weights.len());
    ctx.record_generated(&chosen, loc);
    chosen
}

// === Byte generators ===

/// Uniformly-distributed fixed-size byte array.
///
/// The generic parameter `N` sets the array length at compile time, so
/// the whole result is stack-allocated and recorded as a single trace
/// entry (`[u8; N] = [...]`) rather than N separate `u8` entries.
///
/// # Examples
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0);
/// let key: [u8; 32] = noprop::sample_bytes(&mut ctx);
/// assert_eq!(key.len(), 32);
/// ```
#[track_caller]
pub fn sample_bytes<const N: usize>(ctx: &mut TestCaseContext) -> [u8; N] {
    let loc = Location::caller();
    let bytes = raw_bytes::<N>(ctx);
    ctx.record_generated(&bytes, loc);
    bytes
}

/// Uniformly-distributed `Vec<u8>` of length `len`.
///
/// Use this when the byte-buffer length is known only at runtime
/// (`sample_bytes_vec(ctx, sample_usize_in(ctx, 0..1024))`). The whole
/// buffer is recorded as a single trace entry.
///
/// # Examples
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0);
/// let bytes = noprop::sample_bytes_vec(&mut ctx, 100);
/// assert_eq!(bytes.len(), 100);
/// ```
#[track_caller]
pub fn sample_bytes_vec(ctx: &mut TestCaseContext, len: usize) -> Vec<u8> {
    let loc = Location::caller();
    let mut bytes = vec![0u8; len];
    ctx.fill(&mut bytes);
    ctx.record_generated(&bytes, loc);
    bytes
}

// === Boolean generator ===

/// Uniformly-distributed `bool`.
#[track_caller]
pub fn sample_bool(ctx: &mut TestCaseContext) -> bool {
    let loc = Location::caller();
    // Consume one byte so this primitive shares the "read a fixed-size
    // byte slice" shape with the integer generators.
    let v = raw_bytes::<1>(ctx)[0] & 1 != 0;
    ctx.record_generated(&v, loc);
    v
}

// === Integer generators ===
//
// All primitives draw randomness through `TestCaseContext::fill` (LE bytes ->
// `from_le_bytes`) so that every primitive consumes a fixed-size byte
// slice from the RNG. This keeps every generator compatible with a
// future bytes-based shrink implementation that swaps the RNG for a
// byte reader.

/// Uniformly-distributed `u8`.
#[track_caller]
pub fn sample_u8(ctx: &mut TestCaseContext) -> u8 {
    let loc = Location::caller();
    ctx.set_next_choice_meta(crate::rng::ChoiceMeta::Integer);
    let v = raw_bytes::<1>(ctx)[0];
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `u16`.
#[track_caller]
pub fn sample_u16(ctx: &mut TestCaseContext) -> u16 {
    let loc = Location::caller();
    ctx.set_next_choice_meta(crate::rng::ChoiceMeta::Integer);
    let v = u16::from_le_bytes(raw_bytes(ctx));
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `u32`.
#[track_caller]
pub fn sample_u32(ctx: &mut TestCaseContext) -> u32 {
    let loc = Location::caller();
    ctx.set_next_choice_meta(crate::rng::ChoiceMeta::Integer);
    let v = u32::from_le_bytes(raw_bytes(ctx));
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `u64`.
#[track_caller]
pub fn sample_u64(ctx: &mut TestCaseContext) -> u64 {
    let loc = Location::caller();
    ctx.set_next_choice_meta(crate::rng::ChoiceMeta::Integer);
    let v = u64::from_le_bytes(raw_bytes(ctx));
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `u128`.
#[track_caller]
pub fn sample_u128(ctx: &mut TestCaseContext) -> u128 {
    let loc = Location::caller();
    ctx.set_next_choice_meta(crate::rng::ChoiceMeta::Integer);
    let v = u128::from_le_bytes(raw_bytes(ctx));
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `usize`.
#[track_caller]
pub fn sample_usize(ctx: &mut TestCaseContext) -> usize {
    let loc = Location::caller();
    ctx.set_next_choice_meta(crate::rng::ChoiceMeta::Integer);
    let v = usize::from_le_bytes(raw_bytes(ctx));
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i8`.
#[track_caller]
pub fn sample_i8(ctx: &mut TestCaseContext) -> i8 {
    let loc = Location::caller();
    ctx.set_next_choice_meta(crate::rng::ChoiceMeta::Integer);
    let v = raw_bytes::<1>(ctx)[0] as i8;
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i16`.
#[track_caller]
pub fn sample_i16(ctx: &mut TestCaseContext) -> i16 {
    let loc = Location::caller();
    ctx.set_next_choice_meta(crate::rng::ChoiceMeta::Integer);
    let v = i16::from_le_bytes(raw_bytes(ctx));
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i32`.
#[track_caller]
pub fn sample_i32(ctx: &mut TestCaseContext) -> i32 {
    let loc = Location::caller();
    ctx.set_next_choice_meta(crate::rng::ChoiceMeta::Integer);
    let v = i32::from_le_bytes(raw_bytes(ctx));
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i64`.
#[track_caller]
pub fn sample_i64(ctx: &mut TestCaseContext) -> i64 {
    let loc = Location::caller();
    ctx.set_next_choice_meta(crate::rng::ChoiceMeta::Integer);
    let v = i64::from_le_bytes(raw_bytes(ctx));
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i128`.
#[track_caller]
pub fn sample_i128(ctx: &mut TestCaseContext) -> i128 {
    let loc = Location::caller();
    ctx.set_next_choice_meta(crate::rng::ChoiceMeta::Integer);
    let v = i128::from_le_bytes(raw_bytes(ctx));
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `isize`.
#[track_caller]
pub fn sample_isize(ctx: &mut TestCaseContext) -> isize {
    let loc = Location::caller();
    ctx.set_next_choice_meta(crate::rng::ChoiceMeta::Integer);
    let v = isize::from_le_bytes(raw_bytes(ctx));
    ctx.record_generated(&v, loc);
    v
}

// === Character generators ===
//
// For character subsets beyond the ones below (alphanumeric, hexdigit,
// etc.), compose with `sample_choice` over a byte-string literal, for
// example:
//
//     let d = sample_choice(ctx, b"0123456789") as char;
//     let a = sample_choice(ctx, b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789") as char;

/// Uniformly-distributed `char` over the valid Unicode scalar values
/// (`0..=0x10FFFF`, excluding the surrogate range `0xD800..=0xDFFF`).
///
/// Uses [`sample_with_rejection`] with the shared internal 64-attempt
/// bound: rejection sampling on a 21-bit mask, ~47% per-attempt
/// rejection rate, so `P(all 64 fail) < 10⁻²⁰`.
#[track_caller]
pub fn sample_char(ctx: &mut TestCaseContext) -> char {
    let loc = Location::caller();
    let c = sample_with_rejection(ctx, DEFAULT_MAX_ATTEMPTS, |ctx| {
        let n = u32::from_le_bytes(raw_bytes(ctx)) & 0x1F_FFFF;
        char::from_u32(n)
    });
    ctx.record_generated(&c, loc);
    c
}

/// Uniformly-distributed ASCII `char` (`0x00..=0x7F`, including control
/// characters).
#[track_caller]
pub fn sample_ascii_char(ctx: &mut TestCaseContext) -> char {
    let loc = Location::caller();
    let v = (raw_bytes::<1>(ctx)[0] & 0x7F) as char;
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed printable ASCII `char` (`0x20..=0x7E`, space
/// through `~`, excluding control characters and DEL).
#[track_caller]
pub fn sample_ascii_printable_char(ctx: &mut TestCaseContext) -> char {
    let loc = Location::caller();
    // 95 characters. Use u32 for negligible modulo bias
    // (2^32 mod 95 = 6, so bias factor is at most 1 + 1/45210182).
    let v = (0x20 + u32::from_le_bytes(raw_bytes(ctx)) % 95) as u8 as char;
    ctx.record_generated(&v, loc);
    v
}

// === String generators ===
//
// Length is measured in Unicode code points (equal to `.chars().count()`
// of the returned `String`). Random-length strings compose from
// `sample_usize_in` + one of these primitives, matching the
// `sample_bytes_vec(ctx, len)` shape:
//
//     let n = noprop::sample_usize_in(ctx, 0..=max_len);
//     let s = noprop::sample_string(ctx, n);
//
// A higher-order `sample_string_of(ctx, len, |ctx| ...)` helper is
// deliberately not provided: `(0..len).map(|_| ...).collect()` is
// short enough that a helper would only obscure the imperative
// generator's control flow.

/// Uniformly-distributed `String` of exactly `len` Unicode scalar
/// values. Each code point is produced by calling [`sample_char`] once.
///
/// `len` is a code-point count, not a UTF-8 byte count. The returned
/// string's `.chars().count()` equals `len`; its byte length is up to
/// `4 * len`.
///
/// For random-length strings, wrap this call with [`sample_usize_in`]
/// (see the module-level "String generators" note). For a byte buffer,
/// use [`sample_bytes_vec`]. For a single character, use [`sample_char`].
///
/// # Trace
///
/// One trace entry is recorded per call (`alloc::string::String =
/// "..."`, Rust `Debug` escape). Because [`sample_char`] internally
/// uses a bounded rejection loop, each call to `sample_string`
/// consumes up to `len × 64` internal attempts and, in Recording
/// mode, opens `len` attempt spans in the choice sequence — one per
/// character.
///
/// # Examples
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0);
/// let s = noprop::sample_string(&mut ctx, 10);
/// assert_eq!(s.chars().count(), 10);
/// ```
#[track_caller]
pub fn sample_string(ctx: &mut TestCaseContext, len: usize) -> String {
    let loc = Location::caller();
    let s: String = (0..len).map(|_| sample_char_raw(ctx)).collect();
    ctx.record_generated(&s, loc);
    s
}

/// Uniformly-distributed ASCII `String` of exactly `len` code points
/// (`0x00..=0x7F`, including control characters). Each character is
/// produced by calling [`sample_ascii_char`] once.
///
/// The returned string's byte length equals `len` (ASCII is 1 byte per
/// code point). For printable ASCII only, use
/// [`sample_ascii_printable_string`].
///
/// # Trace
///
/// One trace entry is recorded per call. Unlike [`sample_string`],
/// this primitive uses no internal rejection loop, so it consumes
/// exactly `len` bytes from the RNG and opens no attempt spans.
///
/// # Examples
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0);
/// let s = noprop::sample_ascii_string(&mut ctx, 8);
/// assert_eq!(s.len(), 8);
/// assert!(s.chars().all(|c| c.is_ascii()));
/// ```
#[track_caller]
pub fn sample_ascii_string(ctx: &mut TestCaseContext, len: usize) -> String {
    let loc = Location::caller();
    let s: String = (0..len).map(|_| sample_ascii_char_raw(ctx)).collect();
    ctx.record_generated(&s, loc);
    s
}

/// Uniformly-distributed printable-ASCII `String` of exactly `len`
/// code points (`0x20..=0x7E`, space through `~`). Each character is
/// produced by calling [`sample_ascii_printable_char`] once.
///
/// The returned string's byte length equals `len`. For arbitrary
/// ASCII (including control characters), use [`sample_ascii_string`].
///
/// # Trace
///
/// One trace entry is recorded per call; no attempt spans are opened.
///
/// # Examples
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0);
/// let s = noprop::sample_ascii_printable_string(&mut ctx, 12);
/// assert_eq!(s.len(), 12);
/// assert!(s.chars().all(|c| (0x20..=0x7E).contains(&(c as u32))));
/// ```
#[track_caller]
pub fn sample_ascii_printable_string(ctx: &mut TestCaseContext, len: usize) -> String {
    let loc = Location::caller();
    let s: String = (0..len)
        .map(|_| sample_ascii_printable_char_raw(ctx))
        .collect();
    ctx.record_generated(&s, loc);
    s
}

/// Internal helpers that mirror the public `sample_*_char` primitives
/// but skip the per-character `record_generated` call. Used inside
/// `sample_*_string` so the trace shows one `String` entry rather
/// than `len` `char` entries.
#[track_caller]
fn sample_char_raw(ctx: &mut TestCaseContext) -> char {
    sample_with_rejection(ctx, DEFAULT_MAX_ATTEMPTS, |ctx| {
        let n = u32::from_le_bytes(raw_bytes(ctx)) & 0x1F_FFFF;
        char::from_u32(n)
    })
}

fn sample_ascii_char_raw(ctx: &mut TestCaseContext) -> char {
    (raw_bytes::<1>(ctx)[0] & 0x7F) as char
}

fn sample_ascii_printable_char_raw(ctx: &mut TestCaseContext) -> char {
    (0x20 + u32::from_le_bytes(raw_bytes(ctx)) % 95) as u8 as char
}

// === Floating-point generators ===

/// Uniformly-distributed `f32` in `[min, max)`.
///
/// NaN and infinities are excluded from the output range. To include
/// them (or any specific special value), pick from a fixed set with
/// [`sample_choice`]:
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0);
/// let _x = noprop::sample_choice(
///     &mut ctx,
///     &[f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0],
/// );
/// ```
///
/// For an arbitrary `f32` bit pattern (including NaN, infinities, and
/// subnormals):
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0);
/// let _x = f32::from_bits(noprop::sample_u32(&mut ctx));
/// ```
///
/// For the full finite domain without a specific range, use
/// [`sample_f32`].
///
/// # Panics
///
/// Panics if `min` or `max` is not finite, or if `min >= max`.
#[track_caller]
pub fn sample_f32_in(ctx: &mut TestCaseContext, min: f32, max: f32) -> f32 {
    assert!(
        min.is_finite() && max.is_finite(),
        "sample_f32_in: min and max must be finite"
    );
    assert!(min < max, "sample_f32_in: min must be less than max");
    let loc = Location::caller();
    // Build a 24-bit uniform value in [0, 1): construct a float in
    // [1, 2) by injecting 23 random bits into the mantissa of a fixed
    // exponent, then subtract 1. This is bias-free (every representable
    // value in [0, 1) with 24-bit precision is equally likely).
    let bits = 0x3F80_0000 | (u32::from_le_bytes(raw_bytes(ctx)) >> 9);
    let unit = f32::from_bits(bits) - 1.0;
    let v = min + (max - min) * unit;
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `f64` in `[min, max)`.
///
/// Same conventions as [`sample_f32_in`]: NaN and infinities are excluded from
/// the output. Use [`sample_choice`] to include specific special values, or
/// `f64::from_bits(sample_u64(ctx))` for an arbitrary bit pattern. For the
/// full finite domain without a specific range, use [`sample_f64`].
///
/// # Panics
///
/// Panics if `min` or `max` is not finite, or if `min >= max`.
#[track_caller]
pub fn sample_f64_in(ctx: &mut TestCaseContext, min: f64, max: f64) -> f64 {
    assert!(
        min.is_finite() && max.is_finite(),
        "sample_f64_in: min and max must be finite"
    );
    assert!(min < max, "sample_f64_in: min must be less than max");
    let loc = Location::caller();
    // Same construction as sample_f32 but with 53-bit precision.
    let bits = 0x3FF0_0000_0000_0000 | (u64::from_le_bytes(raw_bytes(ctx)) >> 12);
    let unit = f64::from_bits(bits) - 1.0;
    let v = min + (max - min) * unit;
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed finite `f32` over the full finite domain
/// (excludes `NaN` and `±∞`; includes normals, subnormals, and both
/// signed zeros).
///
/// This is the common shape for roundtrip / serialization property
/// tests, where NaN and infinity are typically outside the format's
/// support. For an arbitrary bit pattern (including NaN / ±∞), use
/// `f32::from_bits(noprop::sample_u32(ctx))` instead. For a specific
/// finite subrange, use [`sample_f32_in`].
///
/// # Implementation
///
/// Rejection sampling over 32-bit patterns via
/// [`sample_with_rejection`]. Only ~0.4 % of `u32` bit patterns decode
/// to non-finite `f32` (all `NaN`s plus `±∞`), so the shared 64-attempt
/// bound is effectively unreachable (`P(all 64 fail) < 10⁻¹⁵²`).
///
/// # Examples
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0);
/// let x = noprop::sample_f32(&mut ctx);
/// assert!(x.is_finite());
/// ```
#[track_caller]
pub fn sample_f32(ctx: &mut TestCaseContext) -> f32 {
    let loc = Location::caller();
    let v = sample_with_rejection(ctx, DEFAULT_MAX_ATTEMPTS, |ctx| {
        let candidate = f32::from_bits(u32::from_le_bytes(raw_bytes(ctx)));
        candidate.is_finite().then_some(candidate)
    });
    ctx.record_generated(&v, loc);
    v
}

/// Uniformly-distributed finite `f64` over the full finite domain.
/// Same conventions and rationale as [`sample_f32`]; the
/// rejection rate over `u64` bit patterns is even lower (~2⁻¹¹ of
/// patterns are non-finite).
///
/// For an arbitrary bit pattern, use
/// `f64::from_bits(noprop::sample_u64(ctx))`. For a specific finite
/// subrange, use [`sample_f64_in`].
///
/// # Examples
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0);
/// let x = noprop::sample_f64(&mut ctx);
/// assert!(x.is_finite());
/// ```
#[track_caller]
pub fn sample_f64(ctx: &mut TestCaseContext) -> f64 {
    let loc = Location::caller();
    let v = sample_with_rejection(ctx, DEFAULT_MAX_ATTEMPTS, |ctx| {
        let candidate = f64::from_bits(u64::from_le_bytes(raw_bytes(ctx)));
        candidate.is_finite().then_some(candidate)
    });
    ctx.record_generated(&v, loc);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assert that `ctx` and `fresh` (built from the same seed) still
    /// produce identical output — used by the "consumes no RNG state"
    /// tests. Compares `fill` outputs rather than the removed
    /// `TestCaseContext::next_u64`.
    fn assert_state_unadvanced(ctx: &mut TestCaseContext, fresh: &mut TestCaseContext) {
        let mut a = [0u8; 8];
        let mut b = [0u8; 8];
        ctx.fill(&mut a);
        fresh.fill(&mut b);
        assert_eq!(a, b, "ctx advanced when it should not have");
    }

    #[test]
    fn primitives_are_deterministic() {
        let mut a = TestCaseContext::new(123);
        let mut b = TestCaseContext::new(123);
        assert_eq!(sample_bool(&mut a), sample_bool(&mut b));
        assert_eq!(sample_u8(&mut a), sample_u8(&mut b));
        assert_eq!(sample_u16(&mut a), sample_u16(&mut b));
        assert_eq!(sample_u32(&mut a), sample_u32(&mut b));
        assert_eq!(sample_u64(&mut a), sample_u64(&mut b));
        assert_eq!(sample_u128(&mut a), sample_u128(&mut b));
        assert_eq!(sample_usize(&mut a), sample_usize(&mut b));
        assert_eq!(sample_i8(&mut a), sample_i8(&mut b));
        assert_eq!(sample_i16(&mut a), sample_i16(&mut b));
        assert_eq!(sample_i32(&mut a), sample_i32(&mut b));
        assert_eq!(sample_i64(&mut a), sample_i64(&mut b));
        assert_eq!(sample_i128(&mut a), sample_i128(&mut b));
        assert_eq!(sample_isize(&mut a), sample_isize(&mut b));
    }

    #[test]
    fn bool_produces_both_values() {
        let mut ctx = TestCaseContext::new(1);
        let (mut t, mut f) = (false, false);
        for _ in 0..64 {
            match sample_bool(&mut ctx) {
                true => t = true,
                false => f = true,
            }
            if t && f {
                return;
            }
        }
        panic!("bool samples covered only one value");
    }

    #[test]
    fn u8_covers_both_halves_of_range() {
        let mut ctx = TestCaseContext::new(1);
        let (mut low, mut high) = (false, false);
        for _ in 0..64 {
            let v = sample_u8(&mut ctx);
            low |= v < 128;
            high |= v >= 128;
            if low && high {
                return;
            }
        }
        panic!("u8 samples covered only one half of the range");
    }

    #[test]
    fn i8_can_be_negative_and_nonnegative() {
        let mut ctx = TestCaseContext::new(1);
        let (mut neg, mut nonneg) = (false, false);
        for _ in 0..64 {
            let v = sample_i8(&mut ctx);
            neg |= v < 0;
            nonneg |= v >= 0;
            if neg && nonneg {
                return;
            }
        }
        panic!("i8 samples covered only one sign");
    }

    #[test]
    fn sample_choice_returns_only_from_slice() {
        let mut ctx = TestCaseContext::new(1);
        let choices = [10, 20, 30];
        for _ in 0..256 {
            assert!(choices.contains(&sample_choice(&mut ctx, &choices)));
        }
    }

    #[test]
    fn sample_choice_can_hit_every_element() {
        let mut ctx = TestCaseContext::new(1);
        let choices = [10, 20, 30];
        let mut seen = [false; 3];
        for _ in 0..256 {
            let v = sample_choice(&mut ctx, &choices);
            let idx = choices.iter().position(|&x| x == v).unwrap();
            seen[idx] = true;
            if seen.iter().all(|&s| s) {
                return;
            }
        }
        panic!("sample_choice did not cover all elements");
    }

    #[test]
    #[should_panic(expected = "empty slice")]
    fn sample_choice_panics_on_empty() {
        let mut ctx = TestCaseContext::new(0);
        let empty: [u32; 0] = [];
        let _ = sample_choice(&mut ctx, &empty);
    }

    #[test]
    fn sample_choice_works_with_clone_only_types() {
        // Verify T: Clone + Debug bound accepts non-Copy types with Debug.
        let mut ctx = TestCaseContext::new(1);
        let choices = vec![String::from("a"), String::from("b"), String::from("c")];
        let picked = sample_choice(&mut ctx, &choices);
        assert!(choices.contains(&picked));
    }

    #[test]
    fn char_generators_are_deterministic() {
        let mut a = TestCaseContext::new(789);
        let mut b = TestCaseContext::new(789);
        assert_eq!(sample_char(&mut a), sample_char(&mut b));
        assert_eq!(sample_ascii_char(&mut a), sample_ascii_char(&mut b));
        assert_eq!(
            sample_ascii_printable_char(&mut a),
            sample_ascii_printable_char(&mut b)
        );
    }

    #[test]
    fn sample_ascii_char_always_in_ascii_range() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..1000 {
            let c = sample_ascii_char(&mut ctx);
            assert!(c.is_ascii());
        }
    }

    #[test]
    fn sample_ascii_printable_char_always_in_range() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..1000 {
            let c = sample_ascii_printable_char(&mut ctx);
            let n = c as u32;
            assert!((0x20..=0x7E).contains(&n));
        }
    }

    // === sample_string / sample_ascii_string / sample_ascii_printable_string ===

    #[test]
    fn sample_string_returns_requested_code_point_count() {
        let mut ctx = TestCaseContext::new(1);
        for len in [0, 1, 7, 32] {
            let s = sample_string(&mut ctx, len);
            assert_eq!(
                s.chars().count(),
                len,
                "sample_string(ctx, {len}) chars().count() differed"
            );
        }
    }

    #[test]
    fn sample_ascii_string_returns_ascii_bytes_of_requested_length() {
        let mut ctx = TestCaseContext::new(1);
        for len in [0, 1, 7, 32] {
            let s = sample_ascii_string(&mut ctx, len);
            assert_eq!(s.chars().count(), len);
            assert_eq!(s.len(), len, "ASCII code point count == byte count");
            assert!(s.is_ascii());
        }
    }

    #[test]
    fn sample_ascii_printable_string_returns_printable_range() {
        let mut ctx = TestCaseContext::new(1);
        for len in [0, 1, 7, 32] {
            let s = sample_ascii_printable_string(&mut ctx, len);
            assert_eq!(s.chars().count(), len);
            assert_eq!(s.len(), len);
            assert!(
                s.chars().all(|c| (0x20..=0x7E).contains(&(c as u32))),
                "found non-printable in {s:?}"
            );
        }
    }

    #[test]
    fn string_generators_are_deterministic() {
        let mut a = TestCaseContext::new(0x517A_2E70);
        let mut b = TestCaseContext::new(0x517A_2E70);
        for len in [0, 3, 16] {
            assert_eq!(sample_string(&mut a, len), sample_string(&mut b, len));
            assert_eq!(
                sample_ascii_string(&mut a, len),
                sample_ascii_string(&mut b, len)
            );
            assert_eq!(
                sample_ascii_printable_string(&mut a, len),
                sample_ascii_printable_string(&mut b, len)
            );
        }
    }

    #[test]
    fn sample_string_zero_length_is_empty() {
        let mut ctx = TestCaseContext::new(1);
        assert_eq!(sample_string(&mut ctx, 0), "");
        assert_eq!(sample_ascii_string(&mut ctx, 0), "");
        assert_eq!(sample_ascii_printable_string(&mut ctx, 0), "");
    }

    #[test]
    fn sample_char_produces_varied_values() {
        // Valid Unicode scalar space is ~1.1M chars, so 256 samples should
        // be nearly all distinct (collision probability is negligible).
        let mut ctx = TestCaseContext::new(1);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            seen.insert(sample_char(&mut ctx));
        }
        assert!(
            seen.len() > 200,
            "sample_char produced too few distinct values: {}",
            seen.len()
        );
    }

    #[test]
    fn float_generators_are_deterministic() {
        let mut a = TestCaseContext::new(999);
        let mut b = TestCaseContext::new(999);
        assert_eq!(
            sample_f32_in(&mut a, 0.0, 1.0),
            sample_f32_in(&mut b, 0.0, 1.0)
        );
        assert_eq!(
            sample_f64_in(&mut a, -100.0, 100.0),
            sample_f64_in(&mut b, -100.0, 100.0)
        );
    }

    #[test]
    fn sample_f32_in_stays_in_range() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..1000 {
            let v = sample_f32_in(&mut ctx, 10.0, 20.0);
            assert!((10.0..20.0).contains(&v), "out of range: {v}");
        }
    }

    #[test]
    fn sample_f64_in_stays_in_range() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..1000 {
            let v = sample_f64_in(&mut ctx, -1.0, 1.0);
            assert!((-1.0..1.0).contains(&v), "out of range: {v}");
        }
    }

    #[test]
    fn sample_f32_in_covers_both_halves_of_range() {
        let mut ctx = TestCaseContext::new(1);
        let (mut low, mut high) = (false, false);
        for _ in 0..64 {
            let v = sample_f32_in(&mut ctx, 0.0, 1.0);
            low |= v < 0.5;
            high |= v >= 0.5;
            if low && high {
                return;
            }
        }
        panic!("sample_f32_in covered only one half of the range");
    }

    #[test]
    #[should_panic(expected = "must be less than")]
    fn sample_f32_in_panics_when_min_equals_max() {
        let mut ctx = TestCaseContext::new(0);
        let _ = sample_f32_in(&mut ctx, 5.0, 5.0);
    }

    #[test]
    #[should_panic(expected = "must be finite")]
    fn sample_f32_in_panics_on_nan() {
        let mut ctx = TestCaseContext::new(0);
        let _ = sample_f32_in(&mut ctx, f32::NAN, 1.0);
    }

    #[test]
    #[should_panic(expected = "must be finite")]
    fn sample_f32_in_panics_on_infinity() {
        let mut ctx = TestCaseContext::new(0);
        let _ = sample_f32_in(&mut ctx, 0.0, f32::INFINITY);
    }

    #[test]
    #[should_panic(expected = "must be finite")]
    fn sample_f64_in_panics_on_nan() {
        let mut ctx = TestCaseContext::new(0);
        let _ = sample_f64_in(&mut ctx, 0.0, f64::NAN);
    }

    // === sample_f32 / sample_f64 (full finite domain) ===

    #[test]
    fn sample_f32_always_returns_finite() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..10_000 {
            let v = sample_f32(&mut ctx);
            assert!(v.is_finite(), "expected finite, got {v:?}");
            assert!(!v.is_nan(), "expected non-NaN, got {v:?}");
        }
    }

    #[test]
    fn sample_f64_always_returns_finite() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..10_000 {
            let v = sample_f64(&mut ctx);
            assert!(v.is_finite(), "expected finite, got {v:?}");
            assert!(!v.is_nan(), "expected non-NaN, got {v:?}");
        }
    }

    #[test]
    fn full_domain_float_generators_are_deterministic() {
        let mut a = TestCaseContext::new(0xF10A_7000);
        let mut b = TestCaseContext::new(0xF10A_7000);
        for _ in 0..64 {
            assert_eq!(sample_f32(&mut a), sample_f32(&mut b));
            assert_eq!(sample_f64(&mut a), sample_f64(&mut b));
        }
    }

    #[test]
    fn sample_f32_covers_both_signs() {
        // ~half of finite f32 patterns are negative (the sign bit is
        // uniform over accepted candidates).
        let mut ctx = TestCaseContext::new(2);
        let (mut pos, mut neg) = (false, false);
        for _ in 0..256 {
            let v = sample_f32(&mut ctx);
            if v > 0.0 {
                pos = true;
            } else if v < 0.0 {
                neg = true;
            }
            if pos && neg {
                return;
            }
        }
        panic!("sample_f32 did not cover both signs");
    }

    // === sample_below ===

    #[test]
    fn sample_below_one_returns_zero_without_drawing() {
        // n == 1 has a single legal value, so no RNG bytes must be consumed.
        let mut ctx = TestCaseContext::new(1);
        let mut fresh = TestCaseContext::new(1);
        assert_eq!(sample_below(&mut ctx, 1), 0);
        assert_state_unadvanced(&mut ctx, &mut fresh);
    }

    #[test]
    fn sample_below_stays_in_range() {
        let mut ctx = TestCaseContext::new(42);
        for _ in 0..10_000 {
            let v = sample_below(&mut ctx, 7);
            assert!(v < 7);
        }
    }

    #[test]
    fn sample_below_hits_every_value() {
        let mut ctx = TestCaseContext::new(42);
        let mut seen = [false; 5];
        for _ in 0..1024 {
            let v = sample_below(&mut ctx, 5) as usize;
            seen[v] = true;
            if seen.iter().all(|&s| s) {
                return;
            }
        }
        panic!("sample_below did not cover all values");
    }

    #[test]
    fn sample_below_is_roughly_uniform_on_non_divisor() {
        // 3 does not divide 2^64, so the sampler must actively reject.
        // Verify that a large batch of draws is roughly balanced.
        let mut ctx = TestCaseContext::new(7);
        let mut counts = [0usize; 3];
        let total = 30_000;
        for _ in 0..total {
            counts[sample_below(&mut ctx, 3) as usize] += 1;
        }
        // Expected ~10_000 per bucket. Allow a very generous slack.
        let expected = total / 3;
        for c in counts {
            assert!(
                c.abs_diff(expected) < expected / 10,
                "bucket count off: {c} vs {expected}"
            );
        }
    }

    #[test]
    fn sample_below_accepts_full_u64_domain() {
        // u64::MAX is odd (not a multiple of 2), so this exercises the
        // rejection path with the widest possible bound.
        let mut ctx = TestCaseContext::new(1);
        let _v = sample_below(&mut ctx, u64::MAX);
    }

    // === sample_usize_in ===

    #[test]
    fn sample_usize_in_exclusive_stays_in_range() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..1000 {
            let v = sample_usize_in(&mut ctx, 10..20);
            assert!((10..20).contains(&v));
        }
    }

    #[test]
    fn sample_usize_in_inclusive_stays_in_range() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..1000 {
            let v = sample_usize_in(&mut ctx, 10..=20);
            assert!((10..=20).contains(&v));
        }
    }

    #[test]
    fn sample_usize_in_single_element_returns_that_element() {
        let mut ctx = TestCaseContext::new(1);
        // 5..=5 is one element; the runner should return it without
        // consuming any RNG.
        let mut fresh = TestCaseContext::new(1);
        assert_eq!(sample_usize_in(&mut ctx, 5..=5), 5);
        assert_state_unadvanced(&mut ctx, &mut fresh);
    }

    #[test]
    fn sample_usize_in_hits_both_endpoints() {
        let mut ctx = TestCaseContext::new(1);
        let (mut lo, mut hi) = (false, false);
        for _ in 0..1024 {
            let v = sample_usize_in(&mut ctx, 0..=3);
            lo |= v == 0;
            hi |= v == 3;
            if lo && hi {
                return;
            }
        }
        panic!("sample_usize_in did not cover both endpoints");
    }

    #[test]
    fn sample_usize_in_full_range_stays_in_range() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..100 {
            let _v = sample_usize_in(&mut ctx, ..);
            // Any usize is in range; just verify no panic.
        }
    }

    #[test]
    fn sample_usize_in_inclusive_up_to_max_stays_in_range() {
        // Exercises the max - lo + 1 arithmetic on the widest non-full
        // range so it must not overflow.
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..100 {
            let v = sample_usize_in(&mut ctx, 1..=usize::MAX);
            assert!(v >= 1);
        }
    }

    #[test]
    fn sample_usize_in_unbounded_end_stays_in_range() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..100 {
            let v = sample_usize_in(&mut ctx, 100..);
            assert!(v >= 100);
        }
    }

    #[test]
    fn sample_usize_in_is_deterministic() {
        let mut a = TestCaseContext::new(999);
        let mut b = TestCaseContext::new(999);
        for _ in 0..64 {
            assert_eq!(
                sample_usize_in(&mut a, 0..137),
                sample_usize_in(&mut b, 0..137)
            );
        }
    }

    #[test]
    #[should_panic(expected = "empty range")]
    fn sample_usize_in_panics_on_empty_exclusive() {
        let mut ctx = TestCaseContext::new(0);
        let _ = sample_usize_in(&mut ctx, 5..5);
    }

    #[test]
    #[should_panic(expected = "empty range")]
    fn sample_usize_in_panics_on_reversed_inclusive() {
        let mut ctx = TestCaseContext::new(0);
        #[expect(clippy::reversed_empty_ranges)]
        let _ = sample_usize_in(&mut ctx, 5..=4);
    }

    #[test]
    #[should_panic(expected = "empty range")]
    fn sample_usize_in_panics_on_zero_exclusive_end() {
        let mut ctx = TestCaseContext::new(0);
        let _ = sample_usize_in(&mut ctx, ..0);
    }

    #[test]
    #[should_panic(expected = "empty range")]
    fn sample_usize_in_panics_on_excluded_max_start() {
        let mut ctx = TestCaseContext::new(0);
        // An excluded start of usize::MAX would need start + 1, which
        // overflows — semantically the range is empty.
        let _ = sample_usize_in(
            &mut ctx,
            (
                std::ops::Bound::Excluded(usize::MAX),
                std::ops::Bound::<usize>::Unbounded,
            ),
        );
    }

    // === Ratio ===

    #[test]
    fn ratio_new_accepts_valid() {
        let ratio = Ratio::new(1, 3);
        assert_eq!(ratio.numerator, 1);
        assert_eq!(ratio.denominator, 3);
        assert_eq!(Ratio::new(0, 5).numerator, 0);
        assert_eq!(Ratio::new(5, 5).denominator, 5);
    }

    #[test]
    #[should_panic(expected = "Ratio::new: denominator must be non-zero")]
    fn ratio_new_panics_on_zero_denominator() {
        let _ = Ratio::new(1, 0);
    }

    #[test]
    #[should_panic(expected = "Ratio::new: denominator must be non-zero")]
    fn ratio_new_panics_on_zero_zero() {
        let _ = Ratio::new(0, 0);
    }

    #[test]
    #[should_panic(expected = "Ratio::new: numerator must not exceed denominator")]
    fn ratio_new_panics_when_numerator_exceeds_denominator() {
        let _ = Ratio::new(2, 1);
    }

    #[test]
    #[should_panic(expected = "Ratio::new: numerator must not exceed denominator")]
    fn ratio_new_panics_when_numerator_exceeds_denominator_larger() {
        let _ = Ratio::new(11, 10);
    }

    #[test]
    fn ratio_one_nth_matches_new_1_over_n() {
        assert_eq!(Ratio::one_nth(2), Ratio::new(1, 2));
        assert_eq!(Ratio::one_nth(3), Ratio::new(1, 3));
        assert_eq!(Ratio::one_nth(10), Ratio::new(1, 10));
        assert_eq!(Ratio::one_nth(100), Ratio::new(1, 100));
    }

    #[test]
    #[should_panic(expected = "Ratio::one_nth: n must be non-zero")]
    fn ratio_one_nth_panics_on_zero() {
        let _ = Ratio::one_nth(0);
    }

    // === sample_ratio ===

    #[test]
    fn sample_ratio_zero_numerator_always_false_and_draws_nothing() {
        let mut ctx = TestCaseContext::new(1);
        let mut fresh = TestCaseContext::new(1);
        for _ in 0..64 {
            assert!(!sample_ratio(&mut ctx, Ratio::new(0, 10)));
        }
        // No RNG bytes consumed.
        assert_state_unadvanced(&mut ctx, &mut fresh);
    }

    #[test]
    fn sample_ratio_full_numerator_always_true_and_draws_nothing() {
        let mut ctx = TestCaseContext::new(1);
        let mut fresh = TestCaseContext::new(1);
        for _ in 0..64 {
            assert!(sample_ratio(&mut ctx, Ratio::new(7, 7)));
        }
        assert_state_unadvanced(&mut ctx, &mut fresh);
    }

    #[test]
    fn sample_ratio_produces_both_outcomes() {
        let mut ctx = TestCaseContext::new(1);
        let (mut t, mut f) = (false, false);
        for _ in 0..256 {
            match sample_ratio(&mut ctx, Ratio::one_nth(2)) {
                true => t = true,
                false => f = true,
            }
            if t && f {
                return;
            }
        }
        panic!("sample_ratio did not produce both outcomes");
    }

    #[test]
    fn sample_ratio_is_deterministic() {
        let mut a = TestCaseContext::new(999);
        let mut b = TestCaseContext::new(999);
        for _ in 0..64 {
            assert_eq!(
                sample_ratio(&mut a, Ratio::new(3, 7)),
                sample_ratio(&mut b, Ratio::new(3, 7))
            );
        }
    }

    #[test]
    fn sample_ratio_biased_matches_expected_frequency() {
        // 1-in-10 draws should sit near 10% out of 10_000 samples.
        let mut ctx = TestCaseContext::new(1);
        let mut trues: usize = 0;
        let total: usize = 10_000;
        for _ in 0..total {
            if sample_ratio(&mut ctx, Ratio::one_nth(10)) {
                trues += 1;
            }
        }
        let expected = total / 10;
        assert!(
            trues.abs_diff(expected) < expected / 2,
            "sample_ratio(Ratio::one_nth(10)) frequency off: {trues}/{total}"
        );
    }

    // === sample_with_boundaries ===

    #[test]
    fn sample_with_boundaries_full_ratio_always_boundary() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..64 {
            let v = sample_with_boundaries(&mut ctx, &[7, 8], Ratio::new(2, 2), sample_u32);
            assert!(v == 7 || v == 8, "unexpected value: {v}");
        }
    }

    #[test]
    fn sample_with_boundaries_zero_ratio_always_uniform() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..64 {
            let v = sample_with_boundaries(&mut ctx, &[7, 8], Ratio::new(0, 3), sample_u32);
            assert!(v != 7 && v != 8, "unexpected boundary value: {v}");
        }
    }

    #[test]
    fn sample_with_boundaries_produces_both_paths() {
        let mut ctx = TestCaseContext::new(1);
        let (mut boundary, mut uniform) = (false, false);
        for _ in 0..256 {
            let v = sample_with_boundaries(&mut ctx, &[u32::MAX], Ratio::one_nth(2), sample_u32);
            if v == u32::MAX {
                boundary = true;
            } else {
                uniform = true;
            }
            if boundary && uniform {
                return;
            }
        }
        panic!("sample_with_boundaries did not take both paths");
    }

    #[test]
    fn sample_with_boundaries_boundary_frequency_matches_ratio() {
        // 1-in-10 boundary draws should sit near 10% out of 10_000
        // samples. A one-element boundary consumes no RNG bytes, so the
        // count measures the ratio branch alone; the uniform path
        // hitting the sentinel is negligible (2^-32 per draw).
        let mut ctx = TestCaseContext::new(1);
        let mut boundary: usize = 0;
        let total: usize = 10_000;
        for _ in 0..total {
            let v = sample_with_boundaries(&mut ctx, &[u32::MAX], Ratio::one_nth(10), sample_u32);
            if v == u32::MAX {
                boundary += 1;
            }
        }
        let expected = total / 10;
        assert!(
            boundary.abs_diff(expected) < expected / 2,
            "sample_with_boundaries(one_nth(10)) frequency off: {boundary}/{total}"
        );
    }

    #[test]
    fn sample_with_boundaries_boundary_path_hits_every_element() {
        // The boundary path must draw uniformly from `boundaries`: a
        // bug that always returns the first element must be caught.
        let mut ctx = TestCaseContext::new(1);
        let mut seen = [false; 2];
        for _ in 0..256 {
            let v = sample_with_boundaries(&mut ctx, &[7, 8], Ratio::new(2, 2), sample_u32);
            match v {
                7 => seen[0] = true,
                8 => seen[1] = true,
                _ => panic!("unexpected value: {v}"),
            }
            if seen.iter().all(|&s| s) {
                return;
            }
        }
        panic!("sample_with_boundaries boundary path did not cover all elements");
    }

    #[test]
    fn sample_with_boundaries_is_deterministic() {
        let mut a = TestCaseContext::new(1234);
        let mut b = TestCaseContext::new(1234);
        for _ in 0..64 {
            assert_eq!(
                sample_with_boundaries(&mut a, &[0, 1], Ratio::one_nth(4), sample_u64),
                sample_with_boundaries(&mut b, &[0, 1], Ratio::one_nth(4), sample_u64)
            );
        }
    }

    #[test]
    fn sample_with_boundaries_recipe_is_byte_equivalent() {
        // The helper must reproduce the hand-written recipe from the
        // boundary-bias evaluation exactly, so rewriting a generator
        // with the helper does not shift the choice sequence.
        let mut a = TestCaseContext::new(42);
        let mut b = TestCaseContext::new(42);
        for _ in 0..256 {
            let hand_written = if sample_usize_in(&mut a, 0..10) == 0 {
                0
            } else {
                sample_u32(&mut a)
            };
            let helper = sample_with_boundaries(&mut b, &[0], Ratio::one_nth(10), sample_u32);
            assert_eq!(hand_written, helper, "streams diverged");
        }
    }

    #[test]
    #[should_panic(expected = "empty boundaries")]
    fn sample_with_boundaries_panics_on_empty_boundaries() {
        let mut ctx = TestCaseContext::new(0);
        let _ = sample_with_boundaries(&mut ctx, &[] as &[u32], Ratio::one_nth(2), sample_u32);
    }

    // === sample_weighted_index ===

    #[test]
    fn sample_weighted_index_stays_in_range() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..1000 {
            let idx = sample_weighted_index(&mut ctx, &[1, 2, 3, 4]);
            assert!(idx < 4);
        }
    }

    #[test]
    fn sample_weighted_index_hits_every_nonzero_index() {
        let mut ctx = TestCaseContext::new(1);
        let weights = [1, 1, 1];
        let mut seen = [false; 3];
        for _ in 0..1024 {
            seen[sample_weighted_index(&mut ctx, &weights)] = true;
            if seen.iter().all(|&s| s) {
                return;
            }
        }
        panic!("sample_weighted_index did not cover all non-zero indices");
    }

    #[test]
    fn sample_weighted_index_skips_zero_weight_slot() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..1000 {
            let idx = sample_weighted_index(&mut ctx, &[3, 0, 5]);
            assert_ne!(idx, 1, "index 1 has weight 0 and must never be picked");
        }
    }

    #[test]
    fn sample_weighted_index_single_nonzero_always_returns_it() {
        let mut ctx = TestCaseContext::new(1);
        for _ in 0..100 {
            assert_eq!(sample_weighted_index(&mut ctx, &[0, 0, 42, 0]), 2);
        }
    }

    #[test]
    fn sample_weighted_index_is_deterministic() {
        let mut a = TestCaseContext::new(123);
        let mut b = TestCaseContext::new(123);
        let weights = [4, 1, 2, 3];
        for _ in 0..64 {
            assert_eq!(
                sample_weighted_index(&mut a, &weights),
                sample_weighted_index(&mut b, &weights)
            );
        }
    }

    #[test]
    fn sample_weighted_index_frequencies_approximate_weights() {
        // Weights 1:2:3 → ~1/6, 2/6, 3/6 of samples.
        let mut ctx = TestCaseContext::new(1);
        let weights = [1, 2, 3];
        let mut counts = [0usize; 3];
        let total = 12_000;
        for _ in 0..total {
            counts[sample_weighted_index(&mut ctx, &weights)] += 1;
        }
        // Expected 2000 / 4000 / 6000. Allow ±30% slack.
        for (i, (&c, &w)) in counts.iter().zip(weights.iter()).enumerate() {
            let expected = total * w as usize / 6;
            assert!(
                c.abs_diff(expected) * 100 < expected * 30,
                "index {i}: got {c}, expected ~{expected}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "empty weights")]
    fn sample_weighted_index_panics_on_empty() {
        let mut ctx = TestCaseContext::new(0);
        let _ = sample_weighted_index(&mut ctx, &[]);
    }

    #[test]
    #[should_panic(expected = "all weights are zero")]
    fn sample_weighted_index_panics_when_all_weights_zero() {
        let mut ctx = TestCaseContext::new(0);
        let _ = sample_weighted_index(&mut ctx, &[0, 0, 0]);
    }

    // === choice sequence record / replay through the sampling primitives ===
    //
    // These tests exercise if / match / loop control flow combined with
    // sample_below, sample_char, sample_with_rejection (as the uniform
    // NonZero recipe), and sample_bytes_vec inside a single recorded
    // case that must replay bit-exactly.

    use crate::rng::{ChoiceSequence, RecordingSession, ReplayError, ReplaySession};

    /// Composite generator that mixes every rejection-loop and variable-length
    /// path: `sample_below` (via `sample_usize_in`), `sample_char`,
    /// an inline uniform-NonZero recipe via `sample_with_rejection`,
    /// `sample_bytes_vec`, plus `if` / `match` / loop control flow.
    /// Returns a shape summary that a strict replay must reproduce
    /// bit-exactly.
    fn composite_case(
        ctx: &mut TestCaseContext,
    ) -> (Vec<char>, Vec<std::num::NonZero<u8>>, Vec<u8>, u32) {
        let branch = sample_usize_in(ctx, 0..3);
        let chars = if branch == 0 {
            Vec::new()
        } else {
            (0..branch).map(|_| sample_char(ctx)).collect()
        };
        let nz_count = sample_usize_in(ctx, 1..=4);
        let mut nzs = Vec::with_capacity(nz_count);
        for _ in 0..nz_count {
            let nz = sample_with_rejection(ctx, DEFAULT_MAX_ATTEMPTS, |ctx| {
                std::num::NonZero::new(sample_u8(ctx))
            });
            nzs.push(nz);
        }
        let buf_len = sample_usize_in(ctx, 0..=16);
        let bytes = sample_bytes_vec(ctx, buf_len);
        let tail = match sample_usize_in(ctx, 0..3) {
            0 => sample_u32(ctx),
            1 => sample_u32(ctx).wrapping_add(1),
            _ => 0,
        };
        (chars, nzs, bytes, tail)
    }

    #[test]
    fn replay_reproduces_composite_generator_bit_exact() {
        for seed in [1u64, 42, 0xDEAD_BEEF, 0xFEED_FACE] {
            let (expected, seq) = RecordingSession::new(seed).run(composite_case);
            let replayed = ReplaySession::new(seq)
                .run(composite_case)
                .expect("replay of same generator must succeed");
            assert_eq!(replayed, expected, "seed {seed:#x}");
        }
    }

    /// Composite generator that mixes the boundary helper's draw
    /// shapes: a degenerate ratio (no bytes), a one-element boundary
    /// (no bytes past the ratio), and a non-degenerate ratio over a
    /// multi-element boundary (two draws), interleaved with a plain
    /// draw. A strict replay must reproduce the shape bit-exactly.
    fn boundary_mix_case(ctx: &mut TestCaseContext) -> (u32, u32, u32) {
        let a = sample_with_boundaries(ctx, &[7, 8], Ratio::new(2, 2), sample_u32);
        let b = sample_with_boundaries(ctx, &[0], Ratio::one_nth(10), sample_u32);
        let c = sample_with_boundaries(ctx, &[u32::MAX, 0], Ratio::one_nth(2), sample_u32);
        (a, b, c)
    }

    #[test]
    fn replay_reproduces_boundary_mix_generator_bit_exact() {
        for seed in [1u64, 42, 0xDEAD_BEEF] {
            let (expected, seq) = RecordingSession::new(seed).run(boundary_mix_case);
            let replayed = ReplaySession::new(seq)
                .run(boundary_mix_case)
                .expect("replay of same generator must succeed");
            assert_eq!(replayed, expected, "seed {seed:#x}");
        }
    }

    #[test]
    fn replay_stops_generator_on_first_structural_mismatch() {
        // Record two 1-byte draws, then replay a generator that first
        // consumes one then asks for a 4-byte draw against the second
        // recorded 1-byte draw. `DrawLengthMismatch` must fire AND the
        // "unreachable" flag must stay false, proving the generator
        // body did not continue past the mismatch.
        let (_, seq) = RecordingSession::new(1).run(|ctx| {
            let _ = sample_u8(ctx);
            let _ = sample_u8(ctx);
        });
        let flag = std::cell::Cell::new(false);
        let result = ReplaySession::new(seq).run(|ctx| {
            let _ = sample_u8(ctx); // matches first recorded draw
            let _ = sample_u32(ctx); // 4 bytes vs recorded 1 → mismatch
            flag.set(true); // must not run
        });
        assert!(
            matches!(
                result,
                Err(ReplayError::DrawLengthMismatch {
                    expected: 1,
                    actual: 4,
                })
            ),
            "unexpected replay result: {result:?}"
        );
        assert!(!flag.get(), "generator continued past replay mismatch");
    }

    #[test]
    fn replay_after_sample_bytes_vec_matches_bytes() {
        // sample_bytes_vec issues a single TestCaseContext::fill of arbitrary length —
        // the recorded draw's length must match at replay time.
        let (recorded, seq) = RecordingSession::new(7).run(|ctx| sample_bytes_vec(ctx, 100));
        let replayed = ReplaySession::new(seq)
            .run(|ctx| sample_bytes_vec(ctx, 100))
            .expect("same-length replay must succeed");
        assert_eq!(replayed, recorded);
    }

    #[test]
    fn recorded_composite_case_matches_plain_prng() {
        // Recording must not shift the byte stream: the composite case
        // recorded from a given seed must equal the same closure run
        // against a plain TestCaseContext::new(seed).
        let seed = 0xABCD_1234u64;
        let (recorded_value, _seq) = RecordingSession::new(seed).run(composite_case);
        let mut prng = TestCaseContext::new(seed);
        let plain_value = composite_case(&mut prng);
        assert_eq!(recorded_value, plain_value);
    }

    #[test]
    fn empty_sequence_replay_succeeds_when_generator_draws_nothing() {
        let seq = ChoiceSequence::default();
        let result = ReplaySession::new(seq).run(|_ctx| 7u32);
        assert_eq!(result, Ok(7));
    }

    // === sample_with_rejection basic behavior ===

    #[test]
    #[should_panic(expected = "max_attempts must be > 0")]
    fn sample_with_rejection_panics_on_zero_max_attempts() {
        let mut ctx = TestCaseContext::new(0);
        let _: u32 = sample_with_rejection(&mut ctx, 0, |_| Some(1));
    }

    #[test]
    fn sample_with_rejection_returns_first_accepted_value() {
        let mut ctx = TestCaseContext::new(0);
        let v = sample_with_rejection(&mut ctx, 8, |_ctx| Some(42u32));
        assert_eq!(v, 42);
    }

    #[test]
    fn sample_with_rejection_skips_rejected_attempts_and_returns_accepted() {
        let mut ctx = TestCaseContext::new(0);
        let counter = std::cell::Cell::new(0usize);
        let v = sample_with_rejection(&mut ctx, 8, |_ctx| {
            let n = counter.get();
            counter.set(n + 1);
            // Accept on the 4th attempt (0-indexed: 3rd retry).
            if n == 3 { Some(n as u32) } else { None }
        });
        assert_eq!(v, 3);
        assert_eq!(counter.get(), 4);
    }

    #[test]
    fn sample_with_rejection_records_nested_spans_in_recording_mode() {
        let (v, seq): ((u8, u8), _) = RecordingSession::new(1).run(|ctx| {
            sample_with_rejection(ctx, 8, |ctx| {
                let x = sample_u8(ctx);
                let y = sample_with_rejection(ctx, 8, |ctx| Some(sample_u8(ctx)));
                Some((x, y))
            })
        });
        assert_eq!(v.0, seq.draws()[0][0]);
        assert_eq!(v.1, seq.draws()[1][0]);
        // Two spans total: outer accepted, inner accepted; inner's parent is outer.
        assert_eq!(seq.spans().len(), 2);
        assert_eq!(seq.spans()[0].parent, None);
        assert_eq!(seq.spans()[0].verdict, AttemptVerdict::Accepted);
        assert_eq!(seq.spans()[1].parent, Some(0));
        assert_eq!(seq.spans()[1].verdict, AttemptVerdict::Accepted);
    }

    #[test]
    fn sample_with_rejection_records_rejected_spans_in_recording_mode() {
        let (v, seq) = RecordingSession::new(2).run(|ctx| {
            let counter = std::cell::Cell::new(0);
            sample_with_rejection(ctx, 8, |_ctx| {
                let n = counter.get();
                counter.set(n + 1);
                if n < 2 { None } else { Some(n as u32) }
            })
        });
        assert_eq!(v, 2);
        // 3 spans: reject, reject, accept.
        assert_eq!(seq.spans().len(), 3);
        assert_eq!(seq.spans()[0].verdict, AttemptVerdict::Rejected);
        assert_eq!(seq.spans()[1].verdict, AttemptVerdict::Rejected);
        assert_eq!(seq.spans()[2].verdict, AttemptVerdict::Accepted);
        assert!(seq.spans().iter().all(|s| s.parent.is_none()));
    }

    #[test]
    fn sample_with_rejection_allows_drawless_rejected_attempts() {
        // A pure predicate over external state — no draws.
        let (v, seq) = RecordingSession::new(1).run(|ctx| {
            let counter = std::cell::Cell::new(0);
            sample_with_rejection(ctx, 4, |_ctx| {
                let n = counter.get();
                counter.set(n + 1);
                if n < 2 { None } else { Some(n as u32) }
            })
        });
        assert_eq!(v, 2);
        assert_eq!(seq.spans().len(), 3);
        for span in seq.spans() {
            assert_eq!(span.start_draw, span.end_draw); // no draws consumed
        }
        assert!(seq.draws().is_empty());
    }
}
