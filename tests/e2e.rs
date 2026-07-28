//! End-to-end integration tests for the noprop runner.
//!
//! These tests exercise the public API as an external user would:
//! everything is referenced via `noprop::` full qualification, with no
//! `use noprop::*` shortcuts, matching the convention documented for
//! all noprop example code.

#[test]
fn run_returns_ok_when_property_holds() -> noprop::Result<()> {
    noprop::Runner {
        seed: 0xDEAD_BEEF,
        cases: 16,
    }
    .run(|rng| {
        let x = noprop::gen_u32(rng);
        assert_eq!(x, x);
    })
}

#[test]
fn run_returns_err_on_failed_assertion() {
    // Property "every u32 is zero" fails almost immediately.
    let result = noprop::Runner {
        seed: 0x1234,
        cases: 64,
    }
    .run(|rng| {
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
        noprop::Runner { seed, cases: 32 }.run(|rng| {
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
fn zero_cases_returns_ok_without_invoking_property() -> noprop::Result<()> {
    let mut invoked = false;
    noprop::Runner { seed: 0, cases: 0 }.run(|_rng| {
        invoked = true;
    })?;
    assert!(!invoked, "property should not be invoked when cases is 0");
    Ok(())
}

#[test]
fn error_debug_output_contains_seed_and_case() {
    let seed = 0xFEED_FACE_C0DE_BABE;
    let result = noprop::Runner { seed, cases: 1 }.run(|_rng| {
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
    let _ = noprop::Runner {
        seed: 0,
        cases: 100,
    }
    .run(|_rng| {
        count += 1;
        if count == 3 {
            panic!("stop here");
        }
    });
    // The failing case counts, but nothing after it runs.
    assert_eq!(count, 3);
}

#[test]
fn generated_values_are_recorded_in_error() {
    let result = noprop::Runner { seed: 42, cases: 1 }.run(|rng| {
        let x = noprop::gen_u32(rng);
        let b = noprop::gen_bool(rng);
        let c = noprop::gen_ascii_char(rng);
        panic!("forced failure with x={x}, b={b}, c={c:?}");
    });

    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 3, "generated: {generated:?}");
    assert_eq!(generated[0].type_name(), "u32");
    assert_eq!(generated[1].type_name(), "bool");
    assert_eq!(generated[2].type_name(), "char");
    // Value repr matches Debug of the value.
    assert!(generated[0].value_repr().parse::<u32>().is_ok());
    // All three calls happen in the same test file.
    assert!(generated[0].location().file().ends_with("e2e.rs"));
}

#[test]
fn generated_trace_is_isolated_per_case() {
    // Generate one value in case 0, then fail in case 1 after generating
    // a different value. The trace should reflect only case 1's values.
    let mut case = 0;
    let result = noprop::Runner { seed: 7, cases: 5 }.run(|rng| {
        if case == 0 {
            let _ = noprop::gen_u64(rng);
        } else {
            let _ = noprop::gen_u16(rng);
            panic!("fail on case {case}");
        }
        case += 1;
    });

    let err = result.expect_err("expected Err");
    assert_eq!(err.case_index(), 1);
    let generated = err.generated();
    assert_eq!(generated.len(), 1, "generated: {generated:?}");
    assert_eq!(generated[0].type_name(), "u16");
}

#[test]
fn error_debug_output_includes_generated_values() {
    let result = noprop::Runner { seed: 42, cases: 1 }.run(|rng| {
        let _ = noprop::gen_u32(rng);
        panic!("boom");
    });
    let err = result.expect_err("expected Err");
    let debug = format!("{err:?}");
    assert!(debug.contains("generated: ["), "debug: {debug}");
    assert!(debug.contains("- u32 ="), "debug: {debug}");
    assert!(debug.contains("(at "), "debug: {debug}");
    assert!(debug.contains("e2e.rs:"), "debug: {debug}");
}

#[test]
fn error_display_output_includes_generated_values() {
    let result = noprop::Runner { seed: 42, cases: 1 }.run(|rng| {
        let _ = noprop::gen_u8(rng);
        panic!("boom");
    });
    let err = result.expect_err("expected Err");
    let display = format!("{err}");
    assert!(display.contains("Generated values:"), "display: {display}");
    assert!(display.contains("- u8 ="), "display: {display}");
}
