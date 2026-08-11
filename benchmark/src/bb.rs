//! Boundary-biased generator wrappers for the detection benchmark.
//!
//! `u32` / `u64` are the only unbounded integer primitives used by the
//! workloads. These wrappers stand in for a hypothetical default-on
//! type-level boundary bias: with a fixed probability the draw is
//! replaced by a choice over a fixed boundary set, and otherwise the
//! draw is uniform. Bounded draws (`sample_usize_in`) and `sample_bool`
//! are intentionally not wrapped, so the wrappers measure what an
//! automatic type-level bias can and cannot reach.

/// 1 in `MIX_DENOMINATOR` draws is a boundary candidate.
const MIX_DENOMINATOR: usize = 16;

/// Draw one `u32`: 1/16 of the time a uniform choice over the
/// type-level boundary set `[0, 1, MAX, MAX - 1]` (`MIN` is 0 for
/// unsigned integers, and `-1` saturates to `MAX`), otherwise uniform
/// over the full range.
pub(crate) fn u32(ctx: &mut noprop::TestCaseContext) -> u32 {
    if noprop::sample_usize_in(ctx, 0..MIX_DENOMINATOR) == 0 {
        noprop::sample_choice(ctx, &[0, 1, u32::MAX, u32::MAX - 1])
    } else {
        noprop::sample_u32(ctx)
    }
}

/// Draw one `u64`: same boundary mix as [`u32()`], over the `u64` set.
pub(crate) fn u64(ctx: &mut noprop::TestCaseContext) -> u64 {
    if noprop::sample_usize_in(ctx, 0..MIX_DENOMINATOR) == 0 {
        noprop::sample_choice(ctx, &[0, 1, u64::MAX, u64::MAX - 1])
    } else {
        noprop::sample_u64(ctx)
    }
}
