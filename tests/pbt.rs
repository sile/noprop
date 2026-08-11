//! Self-hosted property tests for noprop's public sampling API.
//!
//! `Runner` is only the property-test driver in this file, not the
//! system under test. Each test generates parameters across cases and
//! verifies an invariant, differential oracle, round-trip, or
//! metamorphic relation that must hold for every valid generated
//! input.
//!
//! Tests of runner behavior belong in `tests/e2e.rs`. Private
//! implementation tests, intentional error paths, fixed regression
//! witnesses, and statistical convergence checks belong in the
//! corresponding `src/` module.
//!
//! Everything is referenced via `noprop::` full qualification (no
//! `use noprop::*`), matching the convention used across the crate.

use std::cell::Cell;

/// PBT root seed: `"PBT_SEED"` in ASCII (little-endian).
const ROOT_SEED: u64 = 0x4445_4553_5F54_4250;

/// Draw a `usize` biased toward the extreme values that a uniform
/// draw essentially never hits (`0`, `1`, `usize::MAX - 1`,
/// `usize::MAX`), so range boundaries are actually exercised.
fn sample_usize_boundary_biased(ctx: &mut noprop::TestCaseContext) -> usize {
    noprop::sample_with_boundaries(
        ctx,
        &[0, 1, usize::MAX - 1, usize::MAX],
        noprop::Ratio::one_nth(4),
        noprop::sample_usize,
    )
}

#[test]
fn sample_usize_in_stays_within_generated_ranges() -> noprop::TestResult {
    // Verify sample_usize_in respects the inclusion contract for every
    // non-full range form. The full `..` form is handled separately
    // (see sample_usize_in_full_range_matches_sample_usize) because
    // "the returned value is a usize" is vacuous for it.
    //
    // Boundaries (`0`, `1`, `usize::MAX - 1`, `usize::MAX`) are mixed
    // in via sample_usize_boundary_biased so extreme-endpoint cases
    // (empty-exclusive shift, `..0` shift, high-usize ranges) are
    // actually reached; coverage counters below fail the test if any
    // form or any endpoint class is missed.
    const FORMS: usize = 5; // lo..hi / lo..=hi / lo.. / ..hi / ..=hi
    const BOUNDARIES: usize = 4; // 0 / 1 / MAX-1 / MAX

    let form_seen: Cell<[bool; FORMS]> = Cell::new([false; FORMS]);
    let boundary_seen: Cell<[bool; BOUNDARIES]> = Cell::new([false; BOUNDARIES]);

    noprop::Runner::new(ROOT_SEED).run(1024, |ctx| {
        let form = noprop::sample_usize_in(ctx, 0..FORMS);
        let a = sample_usize_boundary_biased(ctx);
        let b = sample_usize_boundary_biased(ctx);
        let (lo_raw, hi_raw) = if a <= b { (a, b) } else { (b, a) };

        let mut forms = form_seen.get();
        forms[form] = true;
        form_seen.set(forms);

        let mut bounds = boundary_seen.get();
        for &v in &[a, b] {
            if v == 0 {
                bounds[0] = true;
            }
            if v == 1 {
                bounds[1] = true;
            }
            if v == usize::MAX - 1 {
                bounds[2] = true;
            }
            if v == usize::MAX {
                bounds[3] = true;
            }
        }
        boundary_seen.set(bounds);

        match form {
            0 => {
                // lo..hi (exclusive): require lo < hi. If a == b, shift
                // one endpoint by 1 (usize::MAX case: shift lo down
                // instead of hi up, to stay in-domain).
                let (lo, hi) = if lo_raw < hi_raw {
                    (lo_raw, hi_raw)
                } else if lo_raw == usize::MAX {
                    (lo_raw - 1, hi_raw)
                } else {
                    (lo_raw, hi_raw + 1)
                };
                let v = noprop::sample_usize_in(ctx, lo..hi);
                assert!(lo <= v && v < hi, "lo..hi: v={v} not in [{lo}, {hi})");
            }
            1 => {
                // lo..=hi (inclusive): a == b is a legal singleton
                // range.
                let (lo, hi) = (lo_raw, hi_raw);
                let v = noprop::sample_usize_in(ctx, lo..=hi);
                assert!(lo <= v && v <= hi, "lo..=hi: v={v} not in [{lo}, {hi}]");
            }
            2 => {
                // lo..
                let lo = lo_raw;
                let v = noprop::sample_usize_in(ctx, lo..);
                assert!(v >= lo, "lo..: v={v} < {lo}");
            }
            3 => {
                // ..hi (exclusive): hi > 0 required, so shift `..0` to
                // `..1` (a legal one-element range).
                let hi = if hi_raw == 0 { 1 } else { hi_raw };
                let v = noprop::sample_usize_in(ctx, ..hi);
                assert!(v < hi, "..hi: v={v} >= {hi}");
            }
            _ => {
                // ..=hi (inclusive): any hi is legal, including 0 and
                // usize::MAX.
                let hi = hi_raw;
                let v = noprop::sample_usize_in(ctx, ..=hi);
                assert!(v <= hi, "..=hi: v={v} > {hi}");
            }
        }
        Ok(())
    })?;

    for (i, hit) in form_seen.get().iter().enumerate() {
        assert!(*hit, "range form index {i} was not exercised");
    }
    for (i, hit) in boundary_seen.get().iter().enumerate() {
        let label = ["0", "1", "usize::MAX - 1", "usize::MAX"][i];
        assert!(*hit, "boundary class {label} was not exercised");
    }
    Ok(())
}
