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

#[test]
fn sample_usize_in_full_range_matches_sample_usize() -> noprop::TestResult {
    // The full range `..` has no non-vacuous inclusion assertion, so
    // cover it with a differential oracle against sample_usize on
    // identical seeds. The follow-up sample_u64 comparison catches
    // regressions that return the same first value but consume a
    // different number of bytes.
    noprop::Runner::new(ROOT_SEED.wrapping_add(1)).run(256, |ctx| {
        let seed = noprop::sample_u64(ctx);
        let mut actual_ctx = noprop::TestCaseContext::new(seed);
        let mut expected_ctx = noprop::TestCaseContext::new(seed);

        let actual = noprop::sample_usize_in(&mut actual_ctx, ..);
        let expected = noprop::sample_usize(&mut expected_ctx);
        assert_eq!(
            actual, expected,
            "seed={seed:#x}: full-range sample_usize_in must match sample_usize"
        );
        assert_eq!(
            noprop::sample_u64(&mut actual_ctx),
            noprop::sample_u64(&mut expected_ctx),
            "seed={seed:#x}: follow-up bytes diverged"
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn sample_usize_in_excluded_start_matches_inclusive_recipe() -> noprop::TestResult {
    // Differential oracle for the excluded start-bound success path
    // (the `checked_add(1)` in sample_usize_in): every excluded-start
    // form must equal its inclusive/half-open equivalent on identical
    // seeds, in value and in follow-up bytes. A regression that shifts
    // the excluded-start offset (e.g. checked_add(1) -> checked_add(2))
    // changes the sampled width and is caught deterministically.
    //
    // `s` is boundary-biased so the extreme arithmetic is exercised
    // (s == 0 -> lo == 1; s == usize::MAX - 1 -> lo == usize::MAX); the
    // s == usize::MAX empty-range panic is covered by the unit test
    // sample_usize_in_panics_on_excluded_max_start. Coverage counters
    // fail the test if any end-bound form is missed.
    const FORMS: usize = 3; // (Excluded(s), Included(e)) / (Excluded(s), Excluded(e)) / (Excluded(s), Unbounded)
    let form_seen: Cell<[bool; FORMS]> = Cell::new([false; FORMS]);

    noprop::Runner::new(ROOT_SEED.wrapping_add(11)).run(1024, |ctx| {
        let form = noprop::sample_usize_in(ctx, 0..FORMS);
        let a = sample_usize_boundary_biased(ctx);
        let b = sample_usize_boundary_biased(ctx);
        let (s, e) = if a <= b { (a, b) } else { (b, a) };

        // Non-empty conditions (lo = s + 1, hi as per end bound):
        //   Included(e): s + 1 <= e        (s < e)
        //   Excluded(e): s + 1 <= e - 1    (s + 2 <= e)
        //   Unbounded:   s + 1 <= usize::MAX (s < usize::MAX)
        let valid = match form {
            0 => s < e,
            1 => s.checked_add(2).is_some_and(|lo| lo <= e),
            _ => s < usize::MAX,
        };
        if !valid {
            return Ok(());
        }

        let mut forms = form_seen.get();
        forms[form] = true;
        form_seen.set(forms);

        let seed = noprop::sample_u64(ctx);
        let mut actual_ctx = noprop::TestCaseContext::new(seed);
        let mut expected_ctx = noprop::TestCaseContext::new(seed);

        let actual = match form {
            0 => noprop::sample_usize_in(
                &mut actual_ctx,
                (std::ops::Bound::Excluded(s), std::ops::Bound::Included(e)),
            ),
            1 => noprop::sample_usize_in(
                &mut actual_ctx,
                (std::ops::Bound::Excluded(s), std::ops::Bound::Excluded(e)),
            ),
            _ => noprop::sample_usize_in(
                &mut actual_ctx,
                (
                    std::ops::Bound::Excluded(s),
                    std::ops::Bound::<usize>::Unbounded,
                ),
            ),
        };
        let expected = match form {
            0 => noprop::sample_usize_in(&mut expected_ctx, s + 1..=e),
            1 => noprop::sample_usize_in(&mut expected_ctx, s + 1..e),
            _ => noprop::sample_usize_in(&mut expected_ctx, s + 1..),
        };

        assert_eq!(actual, expected, "seed={seed:#x} s={s} e={e} form={form}");
        assert_eq!(
            noprop::sample_u64(&mut actual_ctx),
            noprop::sample_u64(&mut expected_ctx),
            "seed={seed:#x} s={s} e={e} form={form}: follow-up bytes diverged"
        );
        Ok(())
    })?;

    for (i, hit) in form_seen.get().iter().enumerate() {
        assert!(*hit, "excluded-start form index {i} was not exercised");
    }
    Ok(())
}

/// Draw a `u64` biased toward the extreme values that a uniform
/// draw essentially never hits (`0`, `1`, `u64::MAX - 1`,
/// `u64::MAX`), so range boundaries are actually exercised.
fn sample_u64_boundary_biased(ctx: &mut noprop::TestCaseContext) -> u64 {
    noprop::sample_with_boundaries(
        ctx,
        &[0, 1, u64::MAX - 1, u64::MAX],
        noprop::Ratio::one_nth(4),
        noprop::sample_u64,
    )
}

#[test]
fn sample_u64_in_stays_within_generated_ranges() -> noprop::TestResult {
    // Verify sample_u64_in respects the inclusion contract for every
    // non-full range form. The full `..` form is handled separately
    // (see sample_u64_in_full_range_matches_sample_u64) because "the
    // returned value is a u64" is vacuous for it.
    //
    // Boundaries (`0`, `1`, `u64::MAX - 1`, `u64::MAX`) are mixed
    // in via sample_u64_boundary_biased so extreme-endpoint cases
    // (empty-exclusive shift, `..0` shift, high-u64 ranges) are
    // actually reached; coverage counters below fail the test if any
    // form or any endpoint class is missed.
    const FORMS: usize = 5; // lo..hi / lo..=hi / lo.. / ..hi / ..=hi
    const BOUNDARIES: usize = 4; // 0 / 1 / MAX-1 / MAX

    let form_seen: Cell<[bool; FORMS]> = Cell::new([false; FORMS]);
    let boundary_seen: Cell<[bool; BOUNDARIES]> = Cell::new([false; BOUNDARIES]);

    noprop::Runner::new(ROOT_SEED).run(1024, |ctx| {
        let form = noprop::sample_usize_in(ctx, 0..FORMS);
        let a = sample_u64_boundary_biased(ctx);
        let b = sample_u64_boundary_biased(ctx);
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
            if v == u64::MAX - 1 {
                bounds[2] = true;
            }
            if v == u64::MAX {
                bounds[3] = true;
            }
        }
        boundary_seen.set(bounds);

        match form {
            0 => {
                // lo..hi (exclusive): require lo < hi. If a == b, shift
                // one endpoint by 1 (u64::MAX case: shift lo down
                // instead of hi up, to stay in-domain).
                let (lo, hi) = if lo_raw < hi_raw {
                    (lo_raw, hi_raw)
                } else if lo_raw == u64::MAX {
                    (lo_raw - 1, hi_raw)
                } else {
                    (lo_raw, hi_raw + 1)
                };
                let v = noprop::sample_u64_in(ctx, lo..hi);
                assert!(lo <= v && v < hi, "lo..hi: v={v} not in [{lo}, {hi})");
            }
            1 => {
                // lo..=hi (inclusive): a == b is a legal singleton
                // range.
                let (lo, hi) = (lo_raw, hi_raw);
                let v = noprop::sample_u64_in(ctx, lo..=hi);
                assert!(lo <= v && v <= hi, "lo..=hi: v={v} not in [{lo}, {hi}]");
            }
            2 => {
                // lo..
                let lo = lo_raw;
                let v = noprop::sample_u64_in(ctx, lo..);
                assert!(v >= lo, "lo..: v={v} < {lo}");
            }
            3 => {
                // ..hi (exclusive): hi > 0 required, so shift `..0` to
                // `..1` (a legal one-element range).
                let hi = if hi_raw == 0 { 1 } else { hi_raw };
                let v = noprop::sample_u64_in(ctx, ..hi);
                assert!(v < hi, "..hi: v={v} >= {hi}");
            }
            _ => {
                // ..=hi (inclusive): any hi is legal, including 0 and
                // u64::MAX.
                let hi = hi_raw;
                let v = noprop::sample_u64_in(ctx, ..=hi);
                assert!(v <= hi, "..=hi: v={v} > {hi}");
            }
        }
        Ok(())
    })?;

    for (i, hit) in form_seen.get().iter().enumerate() {
        assert!(*hit, "range form index {i} was not exercised");
    }
    for (i, hit) in boundary_seen.get().iter().enumerate() {
        let label = ["0", "1", "u64::MAX - 1", "u64::MAX"][i];
        assert!(*hit, "boundary class {label} was not exercised");
    }
    Ok(())
}

#[test]
fn sample_u64_in_full_range_matches_sample_u64() -> noprop::TestResult {
    // The full range `..` has no non-vacuous inclusion assertion, so
    // cover it with a differential oracle against sample_u64 on
    // identical seeds. The follow-up sample_u64 comparison catches
    // regressions that return the same first value but consume a
    // different number of bytes.
    noprop::Runner::new(ROOT_SEED.wrapping_add(2)).run(256, |ctx| {
        let seed = noprop::sample_u64(ctx);
        let mut actual_ctx = noprop::TestCaseContext::new(seed);
        let mut expected_ctx = noprop::TestCaseContext::new(seed);

        let actual = noprop::sample_u64_in(&mut actual_ctx, ..);
        let expected = noprop::sample_u64(&mut expected_ctx);
        assert_eq!(
            actual, expected,
            "seed={seed:#x}: full-range sample_u64_in must match sample_u64"
        );
        assert_eq!(
            noprop::sample_u64(&mut actual_ctx),
            noprop::sample_u64(&mut expected_ctx),
            "seed={seed:#x}: follow-up bytes diverged"
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn sample_u64_in_excluded_start_matches_inclusive_recipe() -> noprop::TestResult {
    // Differential oracle for the excluded start-bound success path
    // (the `checked_add(1)` in sample_u64_in): every excluded-start
    // form must equal its inclusive/half-open equivalent on identical
    // seeds, in value and in follow-up bytes. A regression that shifts
    // the excluded-start offset (e.g. checked_add(1) -> checked_add(2))
    // changes the sampled width and is caught deterministically.
    //
    // `s` is boundary-biased so the extreme arithmetic is exercised
    // (s == 0 -> lo == 1; s == u64::MAX - 1 -> lo == u64::MAX); the
    // s == u64::MAX empty-range panic is covered by the unit test
    // sample_u64_in_panics_on_excluded_max_start. Coverage counters
    // fail the test if any end-bound form is missed.
    const FORMS: usize = 3; // (Excluded(s), Included(e)) / (Excluded(s), Excluded(e)) / (Excluded(s), Unbounded)
    let form_seen: Cell<[bool; FORMS]> = Cell::new([false; FORMS]);

    noprop::Runner::new(ROOT_SEED.wrapping_add(10)).run(1024, |ctx| {
        let form = noprop::sample_usize_in(ctx, 0..FORMS);
        let a = sample_u64_boundary_biased(ctx);
        let b = sample_u64_boundary_biased(ctx);
        let (s, e) = if a <= b { (a, b) } else { (b, a) };

        // Non-empty conditions (lo = s + 1, hi as per end bound):
        //   Included(e): s + 1 <= e        (s < e)
        //   Excluded(e): s + 1 <= e - 1    (s + 2 <= e)
        //   Unbounded:   s + 1 <= u64::MAX (s < u64::MAX)
        let valid = match form {
            0 => s < e,
            1 => s.checked_add(2).is_some_and(|lo| lo <= e),
            _ => s < u64::MAX,
        };
        if !valid {
            return Ok(());
        }

        let mut forms = form_seen.get();
        forms[form] = true;
        form_seen.set(forms);

        let seed = noprop::sample_u64(ctx);
        let mut actual_ctx = noprop::TestCaseContext::new(seed);
        let mut expected_ctx = noprop::TestCaseContext::new(seed);

        let actual = match form {
            0 => noprop::sample_u64_in(
                &mut actual_ctx,
                (std::ops::Bound::Excluded(s), std::ops::Bound::Included(e)),
            ),
            1 => noprop::sample_u64_in(
                &mut actual_ctx,
                (std::ops::Bound::Excluded(s), std::ops::Bound::Excluded(e)),
            ),
            _ => noprop::sample_u64_in(
                &mut actual_ctx,
                (
                    std::ops::Bound::Excluded(s),
                    std::ops::Bound::<u64>::Unbounded,
                ),
            ),
        };
        let expected = match form {
            0 => noprop::sample_u64_in(&mut expected_ctx, s + 1..=e),
            1 => noprop::sample_u64_in(&mut expected_ctx, s + 1..e),
            _ => noprop::sample_u64_in(&mut expected_ctx, s + 1..),
        };

        assert_eq!(actual, expected, "seed={seed:#x} s={s} e={e} form={form}");
        assert_eq!(
            noprop::sample_u64(&mut actual_ctx),
            noprop::sample_u64(&mut expected_ctx),
            "seed={seed:#x} s={s} e={e} form={form}: follow-up bytes diverged"
        );
        Ok(())
    })?;

    for (i, hit) in form_seen.get().iter().enumerate() {
        assert!(*hit, "excluded-start form index {i} was not exercised");
    }
    Ok(())
}

#[test]
fn sample_ratio_matches_explicit_recipe() -> noprop::TestResult {
    // sample_ratio(Ratio::new(n, d)) must equal:
    //   n == 0            => false (drawless)
    //   n == d            => true  (drawless)
    //   otherwise         => sample_usize_in(0..d as usize) < n as usize
    // Verify the value AND the follow-up byte stream on identical
    // seeds, and ensure all three coverage classes are exercised.
    let zero_seen = Cell::new(false);
    let full_seen = Cell::new(false);
    let mid_seen = Cell::new(false);

    noprop::Runner::new(ROOT_SEED.wrapping_add(2)).run(256, |ctx| {
        let denominator = noprop::sample_usize_in(ctx, 1..=32) as u32;
        let numerator = noprop::sample_usize_in(ctx, 0..=denominator as usize) as u32;
        let seed = noprop::sample_u64(ctx);

        if numerator == 0 {
            zero_seen.set(true);
        } else if numerator == denominator {
            full_seen.set(true);
        } else {
            mid_seen.set(true);
        }

        let ratio = noprop::Ratio::new(numerator, denominator);
        let mut actual_ctx = noprop::TestCaseContext::new(seed);
        let mut expected_ctx = noprop::TestCaseContext::new(seed);

        let actual = noprop::sample_ratio(&mut actual_ctx, ratio);
        let expected = if numerator == 0 {
            false
        } else if numerator == denominator {
            true
        } else {
            noprop::sample_usize_in(&mut expected_ctx, 0..denominator as usize) < numerator as usize
        };
        assert_eq!(
            actual, expected,
            "seed={seed:#x} numerator={numerator} denominator={denominator}"
        );
        assert_eq!(
            noprop::sample_u64(&mut actual_ctx),
            noprop::sample_u64(&mut expected_ctx),
            "seed={seed:#x} numerator={numerator} denominator={denominator}: \
             follow-up bytes diverged"
        );
        Ok(())
    })?;

    assert!(zero_seen.get(), "0% ratio was not exercised");
    assert!(full_seen.get(), "100% ratio was not exercised");
    assert!(mid_seen.get(), "middle ratio was not exercised");
    Ok(())
}

#[test]
fn sample_choice_matches_index_recipe() -> noprop::TestResult {
    // sample_choice(choices) must equal choices[sample_usize_in(0..len)].
    // Length 1 is a drawless case (sample_below(1) early-returns
    // without drawing); length >= 2 goes through the sampler.
    // Distinct values guarantee choices[i] uniquely identifies i, so
    // an off-by-one in the index arithmetic is caught even if the
    // byte stream matches.
    let len_one_seen = Cell::new(false);
    let len_many_seen = Cell::new(false);

    noprop::Runner::new(ROOT_SEED.wrapping_add(3)).run(256, |ctx| {
        let len = noprop::sample_usize_in(ctx, 1..=8);
        let choices: Vec<u32> = (0..len as u32).collect();
        let seed = noprop::sample_u64(ctx);

        if len == 1 {
            len_one_seen.set(true);
        } else {
            len_many_seen.set(true);
        }

        let mut actual_ctx = noprop::TestCaseContext::new(seed);
        let mut expected_ctx = noprop::TestCaseContext::new(seed);

        let actual = noprop::sample_choice(&mut actual_ctx, &choices);
        let idx = noprop::sample_usize_in(&mut expected_ctx, 0..choices.len());
        let expected = choices[idx];

        assert_eq!(actual, expected, "seed={seed:#x} len={len}");
        assert_eq!(
            noprop::sample_u64(&mut actual_ctx),
            noprop::sample_u64(&mut expected_ctx),
            "seed={seed:#x} len={len}: follow-up bytes diverged"
        );
        Ok(())
    })?;

    assert!(len_one_seen.get(), "len == 1 (drawless) was not exercised");
    assert!(len_many_seen.get(), "len >= 2 was not exercised");
    Ok(())
}

#[test]
fn sample_weighted_index_matches_cumulative_weight_model() -> noprop::TestResult {
    // sample_weighted_index draws an offset < sum(weights) and returns
    // the index of the weight bucket containing that offset. Verify
    // against a hand-written linear scan on identical seeds. Cover
    // zero-weight slots (must be skipped), single-nonzero vectors,
    // and multi-nonzero vectors.
    let zero_weight_seen = Cell::new(false);
    let single_nonzero_seen = Cell::new(false);
    let multi_nonzero_seen = Cell::new(false);

    noprop::Runner::new(ROOT_SEED.wrapping_add(4)).run(256, |ctx| {
        let len = noprop::sample_usize_in(ctx, 1..=6);
        let mut weights: Vec<u32> = Vec::with_capacity(len);
        for _ in 0..len {
            weights.push(noprop::sample_usize_in(ctx, 0..=8) as u32);
        }
        // Guarantee at least one positive weight so
        // sample_weighted_index does not panic on "all weights zero".
        if weights.iter().all(|&w| w == 0) {
            weights[0] = 1;
        }
        let seed = noprop::sample_u64(ctx);

        let nonzero_count = weights.iter().filter(|&&w| w > 0).count();
        if weights.contains(&0) {
            zero_weight_seen.set(true);
        }
        if nonzero_count == 1 {
            single_nonzero_seen.set(true);
        } else if nonzero_count >= 2 {
            multi_nonzero_seen.set(true);
        }

        let mut actual_ctx = noprop::TestCaseContext::new(seed);
        let mut expected_ctx = noprop::TestCaseContext::new(seed);

        let actual = noprop::sample_weighted_index(&mut actual_ctx, &weights);

        // Explicit model: draw offset in [0, sum), then linear scan.
        // sample_usize_in(0..sum) drives the same sample_below(sum)
        // core, so the byte stream matches.
        let total: u64 = weights.iter().map(|&w| w as u64).sum();
        let offset = noprop::sample_usize_in(&mut expected_ctx, 0..total as usize) as u64;
        let mut pick = offset;
        let mut expected = weights.len();
        for (i, &w) in weights.iter().enumerate() {
            let w = w as u64;
            if pick < w {
                expected = i;
                break;
            }
            pick -= w;
        }

        assert_eq!(actual, expected, "seed={seed:#x} weights={weights:?}");
        assert_eq!(
            noprop::sample_u64(&mut actual_ctx),
            noprop::sample_u64(&mut expected_ctx),
            "seed={seed:#x} weights={weights:?}: follow-up bytes diverged"
        );
        Ok(())
    })?;

    assert!(zero_weight_seen.get(), "zero-weight slot was not exercised");
    assert!(
        single_nonzero_seen.get(),
        "single-nonzero-weight vector was not exercised"
    );
    assert!(
        multi_nonzero_seen.get(),
        "multi-nonzero-weight vector was not exercised"
    );
    Ok(())
}

#[test]
fn sample_with_boundaries_matches_explicit_recipe() -> noprop::TestResult {
    // sample_with_boundaries(bounds, ratio, sample) must equal:
    //   if sample_ratio(ratio) { sample_choice(bounds) } else { sample() }
    // on the same seed and consume the same bytes for the follow-up.
    // Cover ratio 0% / 100% / middle and boundary-slice singleton /
    // multi-element cases.
    let ratio_zero_seen = Cell::new(false);
    let ratio_full_seen = Cell::new(false);
    let ratio_mid_seen = Cell::new(false);
    let bounds_singleton_seen = Cell::new(false);
    let bounds_multi_seen = Cell::new(false);

    noprop::Runner::new(ROOT_SEED.wrapping_add(5)).run(256, |ctx| {
        let denominator = noprop::sample_usize_in(ctx, 1..=16) as u32;
        let numerator = noprop::sample_usize_in(ctx, 0..=denominator as usize) as u32;
        let ratio = noprop::Ratio::new(numerator, denominator);

        let bounds_len = noprop::sample_usize_in(ctx, 1..=4);
        // Distinct values so the boundary branch can be distinguished
        // from the fallback branch by value.
        let boundaries: Vec<u32> = (0..bounds_len as u32).map(|i| i * 100).collect();

        let seed = noprop::sample_u64(ctx);

        if numerator == 0 {
            ratio_zero_seen.set(true);
        } else if numerator == denominator {
            ratio_full_seen.set(true);
        } else {
            ratio_mid_seen.set(true);
        }
        if bounds_len == 1 {
            bounds_singleton_seen.set(true);
        } else {
            bounds_multi_seen.set(true);
        }

        let mut actual_ctx = noprop::TestCaseContext::new(seed);
        let mut expected_ctx = noprop::TestCaseContext::new(seed);

        let actual =
            noprop::sample_with_boundaries(&mut actual_ctx, &boundaries, ratio, noprop::sample_u32);
        let expected = if noprop::sample_ratio(&mut expected_ctx, ratio) {
            noprop::sample_choice(&mut expected_ctx, &boundaries)
        } else {
            noprop::sample_u32(&mut expected_ctx)
        };

        assert_eq!(
            actual, expected,
            "seed={seed:#x} ratio={numerator}/{denominator} boundaries={boundaries:?}"
        );
        assert_eq!(
            noprop::sample_u64(&mut actual_ctx),
            noprop::sample_u64(&mut expected_ctx),
            "seed={seed:#x} ratio={numerator}/{denominator} boundaries={boundaries:?}: \
             follow-up bytes diverged"
        );
        Ok(())
    })?;

    assert!(ratio_zero_seen.get(), "ratio 0% was not exercised");
    assert!(ratio_full_seen.get(), "ratio 100% was not exercised");
    assert!(ratio_mid_seen.get(), "ratio middle was not exercised");
    assert!(
        bounds_singleton_seen.get(),
        "singleton boundaries slice was not exercised"
    );
    assert!(
        bounds_multi_seen.get(),
        "multi-element boundaries slice was not exercised"
    );
    Ok(())
}

#[test]
fn integer_primitives_match_little_endian_bytes() -> noprop::TestResult {
    // Each sample_u* / sample_i* / sample_usize / sample_isize
    // primitive must equal the same-width sample_bytes::<N>
    // reinterpreted via <T>::from_le_bytes on the same seed. This
    // locks the width, endianness, and signed-conversion of every
    // integer adapter deterministically (no statistical thresholds),
    // and the follow-up sample_u64 comparison catches a regression
    // that drifted the byte count.
    //
    // Types are compared in explicit blocks (no macro), so the width
    // and signedness of each adapter read as their own line.
    const USIZE_BYTES: usize = std::mem::size_of::<usize>();
    const ISIZE_BYTES: usize = std::mem::size_of::<isize>();

    noprop::Runner::new(ROOT_SEED.wrapping_add(6)).run(256, |ctx| {
        // Each block derives its own pair of fresh contexts from the
        // outer stream so every type gets the same input regardless of
        // what earlier blocks consumed.

        // u8
        {
            let seed = noprop::sample_u64(ctx);
            let mut a = noprop::TestCaseContext::new(seed);
            let mut b = noprop::TestCaseContext::new(seed);
            assert_eq!(
                noprop::sample_u8(&mut a),
                u8::from_le_bytes(noprop::sample_bytes::<1>(&mut b)),
                "u8: seed={seed:#x}"
            );
            assert_eq!(
                noprop::sample_u64(&mut a),
                noprop::sample_u64(&mut b),
                "u8 follow-up bytes diverged: seed={seed:#x}"
            );
        }
        // u16
        {
            let seed = noprop::sample_u64(ctx);
            let mut a = noprop::TestCaseContext::new(seed);
            let mut b = noprop::TestCaseContext::new(seed);
            assert_eq!(
                noprop::sample_u16(&mut a),
                u16::from_le_bytes(noprop::sample_bytes::<2>(&mut b)),
                "u16: seed={seed:#x}"
            );
            assert_eq!(
                noprop::sample_u64(&mut a),
                noprop::sample_u64(&mut b),
                "u16 follow-up bytes diverged: seed={seed:#x}"
            );
        }
        // u32
        {
            let seed = noprop::sample_u64(ctx);
            let mut a = noprop::TestCaseContext::new(seed);
            let mut b = noprop::TestCaseContext::new(seed);
            assert_eq!(
                noprop::sample_u32(&mut a),
                u32::from_le_bytes(noprop::sample_bytes::<4>(&mut b)),
                "u32: seed={seed:#x}"
            );
            assert_eq!(
                noprop::sample_u64(&mut a),
                noprop::sample_u64(&mut b),
                "u32 follow-up bytes diverged: seed={seed:#x}"
            );
        }
        // u64
        {
            let seed = noprop::sample_u64(ctx);
            let mut a = noprop::TestCaseContext::new(seed);
            let mut b = noprop::TestCaseContext::new(seed);
            assert_eq!(
                noprop::sample_u64(&mut a),
                u64::from_le_bytes(noprop::sample_bytes::<8>(&mut b)),
                "u64: seed={seed:#x}"
            );
            assert_eq!(
                noprop::sample_u64(&mut a),
                noprop::sample_u64(&mut b),
                "u64 follow-up bytes diverged: seed={seed:#x}"
            );
        }
        // u128
        {
            let seed = noprop::sample_u64(ctx);
            let mut a = noprop::TestCaseContext::new(seed);
            let mut b = noprop::TestCaseContext::new(seed);
            assert_eq!(
                noprop::sample_u128(&mut a),
                u128::from_le_bytes(noprop::sample_bytes::<16>(&mut b)),
                "u128: seed={seed:#x}"
            );
            assert_eq!(
                noprop::sample_u64(&mut a),
                noprop::sample_u64(&mut b),
                "u128 follow-up bytes diverged: seed={seed:#x}"
            );
        }
        // usize
        {
            let seed = noprop::sample_u64(ctx);
            let mut a = noprop::TestCaseContext::new(seed);
            let mut b = noprop::TestCaseContext::new(seed);
            assert_eq!(
                noprop::sample_usize(&mut a),
                usize::from_le_bytes(noprop::sample_bytes::<USIZE_BYTES>(&mut b)),
                "usize: seed={seed:#x}"
            );
            assert_eq!(
                noprop::sample_u64(&mut a),
                noprop::sample_u64(&mut b),
                "usize follow-up bytes diverged: seed={seed:#x}"
            );
        }
        // i8
        {
            let seed = noprop::sample_u64(ctx);
            let mut a = noprop::TestCaseContext::new(seed);
            let mut b = noprop::TestCaseContext::new(seed);
            assert_eq!(
                noprop::sample_i8(&mut a),
                i8::from_le_bytes(noprop::sample_bytes::<1>(&mut b)),
                "i8: seed={seed:#x}"
            );
            assert_eq!(
                noprop::sample_u64(&mut a),
                noprop::sample_u64(&mut b),
                "i8 follow-up bytes diverged: seed={seed:#x}"
            );
        }
        // i16
        {
            let seed = noprop::sample_u64(ctx);
            let mut a = noprop::TestCaseContext::new(seed);
            let mut b = noprop::TestCaseContext::new(seed);
            assert_eq!(
                noprop::sample_i16(&mut a),
                i16::from_le_bytes(noprop::sample_bytes::<2>(&mut b)),
                "i16: seed={seed:#x}"
            );
            assert_eq!(
                noprop::sample_u64(&mut a),
                noprop::sample_u64(&mut b),
                "i16 follow-up bytes diverged: seed={seed:#x}"
            );
        }
        // i32
        {
            let seed = noprop::sample_u64(ctx);
            let mut a = noprop::TestCaseContext::new(seed);
            let mut b = noprop::TestCaseContext::new(seed);
            assert_eq!(
                noprop::sample_i32(&mut a),
                i32::from_le_bytes(noprop::sample_bytes::<4>(&mut b)),
                "i32: seed={seed:#x}"
            );
            assert_eq!(
                noprop::sample_u64(&mut a),
                noprop::sample_u64(&mut b),
                "i32 follow-up bytes diverged: seed={seed:#x}"
            );
        }
        // i64
        {
            let seed = noprop::sample_u64(ctx);
            let mut a = noprop::TestCaseContext::new(seed);
            let mut b = noprop::TestCaseContext::new(seed);
            assert_eq!(
                noprop::sample_i64(&mut a),
                i64::from_le_bytes(noprop::sample_bytes::<8>(&mut b)),
                "i64: seed={seed:#x}"
            );
            assert_eq!(
                noprop::sample_u64(&mut a),
                noprop::sample_u64(&mut b),
                "i64 follow-up bytes diverged: seed={seed:#x}"
            );
        }
        // i128
        {
            let seed = noprop::sample_u64(ctx);
            let mut a = noprop::TestCaseContext::new(seed);
            let mut b = noprop::TestCaseContext::new(seed);
            assert_eq!(
                noprop::sample_i128(&mut a),
                i128::from_le_bytes(noprop::sample_bytes::<16>(&mut b)),
                "i128: seed={seed:#x}"
            );
            assert_eq!(
                noprop::sample_u64(&mut a),
                noprop::sample_u64(&mut b),
                "i128 follow-up bytes diverged: seed={seed:#x}"
            );
        }
        // isize
        {
            let seed = noprop::sample_u64(ctx);
            let mut a = noprop::TestCaseContext::new(seed);
            let mut b = noprop::TestCaseContext::new(seed);
            assert_eq!(
                noprop::sample_isize(&mut a),
                isize::from_le_bytes(noprop::sample_bytes::<ISIZE_BYTES>(&mut b)),
                "isize: seed={seed:#x}"
            );
            assert_eq!(
                noprop::sample_u64(&mut a),
                noprop::sample_u64(&mut b),
                "isize follow-up bytes diverged: seed={seed:#x}"
            );
        }
        Ok(())
    })?;
    Ok(())
}

#[test]
fn string_primitives_preserve_generated_length_and_alphabet() -> noprop::TestResult {
    // For each string primitive, verify the length and character-set
    // invariants documented in its rustdoc. Lengths 0, 1, and MAX_LEN
    // are exercised explicitly (mixed in via sample_with_boundaries)
    // since a uniform draw over `0..=MAX_LEN` rarely hits either
    // extreme.
    const MAX_LEN: usize = 64;
    let len_zero_seen = Cell::new(false);
    let len_one_seen = Cell::new(false);
    let len_max_seen = Cell::new(false);

    noprop::Runner::new(ROOT_SEED.wrapping_add(7)).run(256, |ctx| {
        let len =
            noprop::sample_with_boundaries(ctx, &[0, 1, MAX_LEN], noprop::Ratio::one_nth(4), |c| {
                noprop::sample_usize_in(c, 0..=MAX_LEN)
            });

        if len == 0 {
            len_zero_seen.set(true);
        } else if len == 1 {
            len_one_seen.set(true);
        } else if len == MAX_LEN {
            len_max_seen.set(true);
        }

        // sample_string: valid UTF-8 String of `len` Unicode code
        // points. String type already guarantees UTF-8 validity, so
        // the invariant here is chars().count() == len.
        let s1 = noprop::sample_string(ctx, len);
        assert_eq!(s1.chars().count(), len, "sample_string(len={len})");

        // sample_ascii_string: `len` chars, 1 byte per char, is_ascii.
        let s2 = noprop::sample_ascii_string(ctx, len);
        assert_eq!(
            s2.chars().count(),
            len,
            "sample_ascii_string chars count(len={len})"
        );
        assert_eq!(s2.len(), len, "sample_ascii_string byte len(len={len})");
        assert!(
            s2.is_ascii(),
            "sample_ascii_string(len={len}) not ASCII: {s2:?}"
        );

        // sample_ascii_printable_string: `len` chars in 0x20..=0x7E.
        // Assert per-char range rather than is_ascii alone, since the
        // documented alphabet is stricter.
        let s3 = noprop::sample_ascii_printable_string(ctx, len);
        assert_eq!(
            s3.chars().count(),
            len,
            "sample_ascii_printable_string chars(len={len})"
        );
        assert_eq!(
            s3.len(),
            len,
            "sample_ascii_printable_string bytes(len={len})"
        );
        for c in s3.chars() {
            let n = c as u32;
            assert!(
                (0x20..=0x7E).contains(&n),
                "sample_ascii_printable_string(len={len}) non-printable {c:?}: {s3:?}"
            );
        }
        Ok(())
    })?;

    assert!(len_zero_seen.get(), "length 0 was not exercised");
    assert!(len_one_seen.get(), "length 1 was not exercised");
    assert!(len_max_seen.get(), "length MAX_LEN was not exercised");
    Ok(())
}

/// Range shapes the bounded-float property covers. Each variant is
/// generated valid-by-construction: `min.is_finite()`,
/// `max.is_finite()`, `min < max`, and `(max - min).is_finite()`.
#[derive(Debug, Clone, Copy)]
enum FloatRangeClass {
    /// Adjacent representable values (`base`, `base.next_up()`); the
    /// half-open contract has exactly one representable value inside.
    Adjacent,
    /// `(-magnitude, magnitude)` for magnitudes small enough that the
    /// span stays finite.
    CrossZero,
    /// Around signed zero, `MIN_POSITIVE`, and the smallest subnormals.
    Subnormal,
    /// Wide ranges that cover most of the finite domain (span is still
    /// finite).
    Wide,
    /// Two same-sign finite draws sorted into `min < max`.
    General,
}

const FLOAT_RANGE_CLASSES: [FloatRangeClass; 5] = [
    FloatRangeClass::Adjacent,
    FloatRangeClass::CrossZero,
    FloatRangeClass::Subnormal,
    FloatRangeClass::Wide,
    FloatRangeClass::General,
];

/// Valid-by-construction f32 range for the given class, or `None` if
/// the underlying draw hit an edge that would make the class produce
/// an invalid range (e.g. `Adjacent` when base is `f32::MAX`). The
/// rejection rate is bounded and small, so every class still hits its
/// coverage target within the test's case budget.
fn generate_f32_range(
    ctx: &mut noprop::TestCaseContext,
    class: FloatRangeClass,
) -> Option<(f32, f32)> {
    match class {
        FloatRangeClass::Adjacent => {
            let base = noprop::sample_f32(ctx);
            if base == f32::MAX {
                return None;
            }
            Some((base, base.next_up()))
        }
        FloatRangeClass::CrossZero => {
            let m = noprop::sample_f32(ctx).abs();
            if m == 0.0 || m >= f32::MAX / 2.0 {
                return None;
            }
            Some((-m, m))
        }
        FloatRangeClass::Subnormal => {
            let candidates: &[(f32, f32)] = &[
                (0.0, f32::MIN_POSITIVE),
                (0.0, f32::from_bits(1)),
                (f32::from_bits(1), f32::MIN_POSITIVE),
                (-f32::MIN_POSITIVE, 0.0),
                (-f32::MIN_POSITIVE, f32::MIN_POSITIVE),
            ];
            let idx = noprop::sample_usize_in(ctx, 0..candidates.len());
            Some(candidates[idx])
        }
        FloatRangeClass::Wide => {
            let candidates: &[(f32, f32)] = &[
                (0.0, f32::MAX),
                (-f32::MAX / 2.0, f32::MAX / 2.0),
                (-1.0e30, 1.0e30),
            ];
            let idx = noprop::sample_usize_in(ctx, 0..candidates.len());
            Some(candidates[idx])
        }
        FloatRangeClass::General => {
            let a = noprop::sample_f32(ctx);
            let b = noprop::sample_f32(ctx);
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            if lo == hi {
                if hi == f32::MAX {
                    Some((lo.next_down(), hi))
                } else {
                    Some((lo, hi.next_up()))
                }
            } else {
                Some((lo, hi))
            }
        }
    }
}

/// f64 mirror of `generate_f32_range`.
fn generate_f64_range(
    ctx: &mut noprop::TestCaseContext,
    class: FloatRangeClass,
) -> Option<(f64, f64)> {
    match class {
        FloatRangeClass::Adjacent => {
            let base = noprop::sample_f64(ctx);
            if base == f64::MAX {
                return None;
            }
            Some((base, base.next_up()))
        }
        FloatRangeClass::CrossZero => {
            let m = noprop::sample_f64(ctx).abs();
            if m == 0.0 || m >= f64::MAX / 2.0 {
                return None;
            }
            Some((-m, m))
        }
        FloatRangeClass::Subnormal => {
            let candidates: &[(f64, f64)] = &[
                (0.0, f64::MIN_POSITIVE),
                (0.0, f64::from_bits(1)),
                (f64::from_bits(1), f64::MIN_POSITIVE),
                (-f64::MIN_POSITIVE, 0.0),
                (-f64::MIN_POSITIVE, f64::MIN_POSITIVE),
            ];
            let idx = noprop::sample_usize_in(ctx, 0..candidates.len());
            Some(candidates[idx])
        }
        FloatRangeClass::Wide => {
            let candidates: &[(f64, f64)] = &[
                (0.0, f64::MAX),
                (-f64::MAX / 2.0, f64::MAX / 2.0),
                (-1.0e300, 1.0e300),
            ];
            let idx = noprop::sample_usize_in(ctx, 0..candidates.len());
            Some(candidates[idx])
        }
        FloatRangeClass::General => {
            let a = noprop::sample_f64(ctx);
            let b = noprop::sample_f64(ctx);
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            if lo == hi {
                if hi == f64::MAX {
                    Some((lo.next_down(), hi))
                } else {
                    Some((lo, hi.next_up()))
                }
            } else {
                Some((lo, hi))
            }
        }
    }
}

#[test]
fn bounded_float_primitives_stay_in_half_open_ranges_f32() -> noprop::TestResult {
    // For every FloatRangeClass, verify sample_f32_in returns a
    // finite value in `[min, max)`. Range generation is
    // valid-by-construction and never depends on sample_f32_in
    // itself, so a defect in sample_f32_in cannot silently narrow the
    // input range. Assertion messages include the bit patterns of
    // min / max / value so signed-zero, subnormal, and adjacent-value
    // failures are diagnosable from the failure report.
    let seen: Cell<[bool; FLOAT_RANGE_CLASSES.len()]> =
        Cell::new([false; FLOAT_RANGE_CLASSES.len()]);

    noprop::Runner::new(ROOT_SEED.wrapping_add(8)).run(1024, |ctx| {
        let idx = noprop::sample_usize_in(ctx, 0..FLOAT_RANGE_CLASSES.len());
        let class = FLOAT_RANGE_CLASSES[idx];

        let Some((min, max)) = generate_f32_range(ctx, class) else {
            return Ok(());
        };

        // Range-generator invariants — separate assertions so a
        // range-gen defect is distinguishable from a sample_f32_in
        // defect in the failure message.
        assert!(
            min.is_finite() && max.is_finite(),
            "class={class:?}: non-finite endpoint min={min:e} max={max:e}"
        );
        assert!(min < max, "class={class:?}: min={min:e} not < max={max:e}");
        assert!(
            (max - min).is_finite(),
            "class={class:?}: max - min overflowed to inf (min={min:e}, max={max:e})"
        );

        let value = noprop::sample_f32_in(ctx, min, max);
        assert!(
            value.is_finite() && min <= value && value < max,
            "class={class:?} min={min:e} ({:#010x}) max={max:e} ({:#010x}) \
             value={value:e} ({:#010x})",
            min.to_bits(),
            max.to_bits(),
            value.to_bits()
        );

        let mut s = seen.get();
        s[idx] = true;
        seen.set(s);
        Ok(())
    })?;

    for (i, hit) in seen.get().iter().enumerate() {
        assert!(*hit, "class {:?} was not exercised", FLOAT_RANGE_CLASSES[i]);
    }
    Ok(())
}

#[test]
fn bounded_float_primitives_stay_in_half_open_ranges_f64() -> noprop::TestResult {
    // f64 mirror of the f32 test; kept as a separate function so a
    // failure in one type does not obscure the other in the failure
    // report.
    let seen: Cell<[bool; FLOAT_RANGE_CLASSES.len()]> =
        Cell::new([false; FLOAT_RANGE_CLASSES.len()]);

    noprop::Runner::new(ROOT_SEED.wrapping_add(9)).run(1024, |ctx| {
        let idx = noprop::sample_usize_in(ctx, 0..FLOAT_RANGE_CLASSES.len());
        let class = FLOAT_RANGE_CLASSES[idx];

        let Some((min, max)) = generate_f64_range(ctx, class) else {
            return Ok(());
        };

        assert!(
            min.is_finite() && max.is_finite(),
            "class={class:?}: non-finite endpoint min={min:e} max={max:e}"
        );
        assert!(min < max, "class={class:?}: min={min:e} not < max={max:e}");
        assert!(
            (max - min).is_finite(),
            "class={class:?}: max - min overflowed to inf (min={min:e}, max={max:e})"
        );

        let value = noprop::sample_f64_in(ctx, min, max);
        assert!(
            value.is_finite() && min <= value && value < max,
            "class={class:?} min={min:e} ({:#018x}) max={max:e} ({:#018x}) \
             value={value:e} ({:#018x})",
            min.to_bits(),
            max.to_bits(),
            value.to_bits()
        );

        let mut s = seen.get();
        s[idx] = true;
        seen.set(s);
        Ok(())
    })?;

    for (i, hit) in seen.get().iter().enumerate() {
        assert!(*hit, "class {:?} was not exercised", FLOAT_RANGE_CLASSES[i]);
    }
    Ok(())
}
