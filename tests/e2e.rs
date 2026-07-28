//! End-to-end integration tests for the noprop runner.
//!
//! These tests exercise the public API as an external user would:
//! everything is referenced via `noprop::` full qualification, with no
//! `use noprop::*` shortcuts, matching the convention documented for
//! all noprop example code.

#[test]
fn run_returns_ok_when_property_holds() -> noprop::Result<()> {
    noprop::Runner::new(0xDEAD_BEEF).with_cases(16).run(|rng| {
        let x = noprop::gen_u32(rng);
        assert_eq!(x, x);
    })
}

#[test]
fn run_returns_err_on_failed_assertion() {
    // Property "every u32 is zero" fails almost immediately.
    let result = noprop::Runner::new(0x1234).with_cases(64).run(|rng| {
        let x = noprop::gen_u32(rng);
        assert_eq!(x, 0, "expected zero, got {x}");
    });

    let err = result.expect_err("expected Err, got Ok");
    assert_eq!(err.seed(), 0x1234);
    assert!(err.case_index() < 64);
}

#[test]
fn same_seed_reproduces_same_failure() {
    let seed = 0xABCD_1234_5678_9ABC;

    let run = || {
        noprop::Runner::new(seed).with_cases(32).run(|rng| {
            let x = noprop::gen_u32(rng);
            // Roughly half of cases fail — enough to guarantee an Err
            // within 32 cases with vanishing probability of Ok.
            assert!(x < 0x8000_0000, "high bit set: {x:#010x}");
        })
    };

    let a = run();
    let b = run();

    match (a, b) {
        (Err(a), Err(b)) => {
            assert_eq!(a.seed(), b.seed());
            assert_eq!(a.case_index(), b.case_index());
        }
        other => panic!("expected two matching Err results, got {other:?}"),
    }
}

#[test]
fn default_case_count_is_256() -> noprop::Result<()> {
    let mut count = 0usize;
    noprop::Runner::new(0).run(|_rng| {
        count += 1;
    })?;
    assert_eq!(count, 256);
    Ok(())
}

#[test]
fn with_cases_overrides_default() -> noprop::Result<()> {
    let mut count = 0usize;
    noprop::Runner::new(0).with_cases(7).run(|_rng| {
        count += 1;
    })?;
    assert_eq!(count, 7);
    Ok(())
}

#[test]
fn error_debug_output_contains_seed_and_case() {
    let seed = 0xFEED_FACE_C0DE_BABE;
    let result = noprop::Runner::new(seed).with_cases(1).run(|_rng| {
        panic!("boom");
    });
    let err = result.expect_err("expected panic to become Err");
    let debug = format!("{err:?}");
    assert!(debug.contains("0xfeedfacec0debabe"), "debug: {debug}");
    assert!(debug.contains("case_index: 0"), "debug: {debug}");
    assert!(debug.contains("boom"), "debug: {debug}");
}

#[test]
fn subsequent_cases_are_skipped_after_failure() {
    let mut count = 0usize;
    let _ = noprop::Runner::new(0).with_cases(100).run(|_rng| {
        count += 1;
        if count == 3 {
            panic!("stop here");
        }
    });
    // The failing case counts, but nothing after it runs.
    assert_eq!(count, 3);
}
