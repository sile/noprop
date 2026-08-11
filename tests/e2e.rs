//! End-to-end tests for noprop's observable runner behavior.
//!
//! Put a test here when `Runner` or the execution pipeline is part of
//! the system under test: case execution, rejection, failure reporting,
//! generated traces, statistics, reproducibility, or feedback-guided
//! search.
//!
//! Properties where `Runner` merely drives generated inputs for a
//! sampling primitive belong in `tests/pbt.rs`. Private implementation
//! tests and minimized regression witnesses belong in the corresponding
//! `src/` module.
//!
//! Everything is referenced via `noprop::` full qualification, with no
//! `use noprop::*` shortcuts, matching the convention documented for
//! all noprop example code.

#[test]
fn run_returns_ok_when_property_holds() -> noprop::TestResult {
    noprop::Runner::new(0xDEAD_BEEF).run(16, |ctx| {
        let x = noprop::sample_u32(ctx);
        assert_eq!(x, x);
        Ok(())
    })?;
    Ok(())
}

#[test]
fn test_result_chains_config_and_runner_via_question_mark() {
    // A `#[test] -> noprop::TestResult` fn composes
    // seed_from_env_or_time (io::Error-based) and Runner::run
    // (RunError-based) through `?`. This exercises the type-level
    // composition on the success path; the failure paths of each
    // component are covered independently (see e.g.
    // run_returns_err_on_failed_assertion for the runner side and
    // src/seed.rs's parse tests for the config side).
    let result: noprop::TestResult = (|| {
        let seed = noprop::seed_from_env_or_time("NOPROP_E2E_ABSOLUTELY_UNSET_SEED_7C4A_1B2D")?;
        noprop::Runner::new(seed).run(4, |_ctx| Ok(()))?;
        Ok(())
    })();
    assert!(
        result.is_ok(),
        "fallback config + passing property must succeed"
    );
}

#[test]
fn run_returns_err_on_failed_assertion() {
    // Property "every u32 is zero" fails almost immediately.
    let result = noprop::Runner::new(0x1234).run(64, |ctx| {
        let x = noprop::sample_u32(ctx);
        assert_eq!(x, 0, "expected zero, got {x}");
        Ok(())
    });

    let err = result.expect_err("expected Err, got Ok");
    assert_eq!(err.seed(), 0x1234);
    assert!(err.case_index() < 64);
    assert_eq!(
        err.kind(),
        noprop::RunErrorKind::PropertyFailure,
        "a property failure must be classified as PropertyFailure"
    );
}

#[test]
fn run_budget_is_per_call() {
    // The same runner can be re-run with different case budgets: each
    // run's failure must carry that run's budget in the reproduce hint.
    let mut runner = noprop::Runner::new(0xABCD);
    let failing = |ctx: &mut noprop::TestCaseContext| {
        let _ = noprop::sample_u32(ctx);
        if true {
            panic!("deterministic failure");
        }
        Ok(())
    };

    let err = runner.run(16, failing).expect_err("first run must fail");
    assert!(
        format!("{err}").contains("Runner::new(0x000000000000abcd).run(16, |ctx| ...)"),
        "hint must carry the first run's budget: {err}"
    );

    let err = runner.run(32, failing).expect_err("second run must fail");
    assert!(
        format!("{err}").contains("Runner::new(0x000000000000abcd).run(32, |ctx| ...)"),
        "hint must carry the second run's budget, not the first: {err}"
    );
}

#[test]
fn too_many_rejections_is_classified_by_kind() {
    // The runner gives up with TooManyRejections; `kind()` must report
    // it without string-matching the Display output.
    let err = noprop::Runner::new(1)
        .run(8, |ctx| {
            ctx.reject_case();
        })
        .expect_err("always-rejecting property must hit the rejection cap");
    assert_eq!(
        err.kind(),
        noprop::RunErrorKind::TooManyRejections,
        "rejection-cap exhaustion must be classified as TooManyRejections"
    );
}

#[test]
fn same_seed_reproduces_same_failure() {
    let seed = 0xABCD_1234_5678_9ABC;

    let run = || {
        noprop::Runner::new(seed).run(32, |ctx| {
            let x = noprop::sample_u32(ctx);
            // Roughly half of cases fail — enough to guarantee an
            // Err within 32 cases with vanishing probability of Ok.
            assert!(x < 0x8000_0000, "high bit set: {x:#010x}");
            Ok(())
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
fn zero_cases_returns_ok_without_invoking_property() -> noprop::TestResult {
    let invoked = std::cell::Cell::new(false);
    noprop::Runner::new(0).run(0, |_ctx| {
        invoked.set(true);
        Ok(())
    })?;
    assert!(
        !invoked.get(),
        "property should not be invoked when cases is 0"
    );
    Ok(())
}

#[test]
fn error_debug_output_contains_seed_and_case() {
    let seed = 0xFEED_FACE_C0DE_BABE;
    let result = noprop::Runner::new(seed).run(1, |_ctx| {
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
    // Count cases via a Cell so that the property closure stays a
    // pure `Fn`. Panic on the third invocation and verify the runner
    // stopped there (no fourth invocation).
    let count = std::cell::Cell::new(0usize);
    let _ = noprop::Runner::new(0).run(100, |_ctx| {
        let n = count.get() + 1;
        count.set(n);
        if n == 3 {
            panic!("stop here");
        }
        Ok(())
    });
    assert_eq!(count.get(), 3);
}

#[test]
fn generated_values_are_recorded_in_error() {
    let result = noprop::Runner::new(42).run(1, |ctx| {
        let x = noprop::sample_u32(ctx);
        let b = noprop::sample_bool(ctx);
        let c = noprop::sample_ascii_char(ctx);
        panic!("forced failure with x={x}, b={b}, c={c:?}");
    });

    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 3, "generated: {generated:?}");
    assert_eq!(generated[0].type_name(), "u32");
    assert_eq!(generated[1].type_name(), "bool");
    assert_eq!(generated[2].type_name(), "char");
    // Value repr matches Debug of the value.
    assert!(!generated[0].is_elided());
    assert!(generated[0].value_repr().unwrap().parse::<u32>().is_ok());
    // All three calls happen in the same test file.
    assert!(generated[0].location().file().ends_with("e2e.rs"));
}

#[test]
fn generated_trace_dedups_same_location_run() {
    // Generate many values at a single call site inside a loop; the
    // trace should keep only the head (8) + elision marker (1) + tail
    // (8) = 17 entries.
    let result = noprop::Runner::new(1).run(1, |ctx| {
        for _ in 0..100 {
            let _ = noprop::sample_u8(ctx);
        }
        panic!("fail after loop");
    });

    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 17, "generated: {generated:?}");

    // First 8 are value entries.
    for (i, entry) in generated.iter().take(8).enumerate() {
        assert!(!entry.is_elided(), "entry {i} should be a value");
        assert_eq!(entry.type_name(), "u8");
    }

    // Middle entry is an elision marker for 100 - 16 = 84 skipped values.
    assert!(generated[8].is_elided());
    assert_eq!(generated[8].elided_count(), Some(84));

    // Last 8 are value entries again.
    for (i, entry) in generated.iter().skip(9).enumerate() {
        assert!(!entry.is_elided(), "entry {} should be a value", i + 9);
        assert_eq!(entry.type_name(), "u8");
    }
}

#[test]
fn generated_trace_does_not_dedup_below_head_plus_tail() {
    // With HEAD + TAIL = 16 slots, a run of exactly 16 same-location
    // entries fits without elision.
    let result = noprop::Runner::new(1).run(1, |ctx| {
        for _ in 0..16 {
            let _ = noprop::sample_u8(ctx);
        }
        panic!("fail after loop");
    });

    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 16, "generated: {generated:?}");
    assert!(generated.iter().all(|e| !e.is_elided()));
}

#[test]
fn generated_trace_treats_different_locations_independently() {
    // Two adjacent same-location runs — a small one, then a large one.
    // Each run is deduped independently.
    let result = noprop::Runner::new(1).run(1, |ctx| {
        for _ in 0..3 {
            let _ = noprop::sample_u8(ctx);
        }
        for _ in 0..100 {
            let _ = noprop::sample_u16(ctx);
        }
        panic!("fail after loops");
    });

    let err = result.expect_err("expected Err");
    let generated = err.generated();
    // 3 u8 entries + (8 u16 head + 1 elision + 8 u16 tail) = 20
    assert_eq!(generated.len(), 20, "generated: {generated:?}");
    for entry in generated.iter().take(3) {
        assert_eq!(entry.type_name(), "u8");
        assert!(!entry.is_elided());
    }
    for entry in generated.iter().skip(3).take(8) {
        assert_eq!(entry.type_name(), "u16");
        assert!(!entry.is_elided());
    }
    assert!(generated[11].is_elided());
    assert_eq!(generated[11].elided_count(), Some(84));
    for entry in generated.iter().skip(12) {
        assert_eq!(entry.type_name(), "u16");
        assert!(!entry.is_elided());
    }
}

#[test]
fn generated_trace_is_isolated_per_case() {
    // Generate one value in case 0, then fail in case 1 after generating
    // a different value. The trace should reflect only case 1's values.
    // Cell keeps the closure a pure `Fn` while still stepping through
    // per-iteration branches.
    let case = std::cell::Cell::new(0usize);
    let result = noprop::Runner::new(7).run(5, |ctx| {
        let c = case.get();
        if c == 0 {
            let _ = noprop::sample_u64(ctx);
        } else {
            let _ = noprop::sample_u16(ctx);
            panic!("fail on case {c}");
        }
        case.set(c + 1);
        Ok(())
    });

    let err = result.expect_err("expected Err");
    assert_eq!(err.case_index(), 1);
    let generated = err.generated();
    assert_eq!(generated.len(), 1, "generated: {generated:?}");
    assert_eq!(generated[0].type_name(), "u16");
}

#[test]
fn error_debug_output_includes_generated_values() {
    let result = noprop::Runner::new(42).run(1, |ctx| {
        let _ = noprop::sample_u32(ctx);
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
fn sample_bytes_records_the_array_as_one_trace_entry() {
    let result = noprop::Runner::new(5).run(1, |ctx| {
        let _key: [u8; 16] = noprop::sample_bytes(ctx);
        panic!("stop");
    });
    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 1);
    assert_eq!(generated[0].type_name(), "[u8; 16]");
}

#[test]
fn sample_bytes_vec_records_the_vec_as_one_trace_entry() {
    let result = noprop::Runner::new(5).run(1, |ctx| {
        let _buf = noprop::sample_bytes_vec(ctx, 42);
        panic!("stop");
    });
    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 1);
    assert_eq!(generated[0].type_name(), "alloc::vec::Vec<u8>");
}

#[test]
fn error_display_output_includes_generated_values() {
    let result = noprop::Runner::new(42).run(1, |ctx| {
        let _ = noprop::sample_u8(ctx);
        panic!("boom");
    });
    let err = result.expect_err("expected Err");
    let display = format!("{err}");
    assert!(display.contains("Generated values:"), "display: {display}");
    assert!(display.contains("- u8 ="), "display: {display}");
}

#[test]
fn sample_usize_in_records_only_the_chosen_value() {
    // Rejection sampling can consume several u64 draws internally, but
    // only the final chosen value must appear in the trace.
    let result = noprop::Runner::new(5).run(1, |ctx| {
        let _v = noprop::sample_usize_in(ctx, 0..7);
        panic!("stop");
    });
    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 1, "generated: {generated:?}");
    assert_eq!(generated[0].type_name(), "usize");
    let repr = generated[0].value_repr().unwrap();
    let v: usize = repr.parse().unwrap();
    assert!(v < 7);
}

#[test]
fn sample_ratio_records_only_the_chosen_bool() {
    let result = noprop::Runner::new(5).run(1, |ctx| {
        let _b = noprop::sample_ratio(ctx, noprop::Ratio::one_nth(3));
        panic!("stop");
    });
    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 1, "generated: {generated:?}");
    assert_eq!(generated[0].type_name(), "bool");
}

#[test]
fn sample_with_boundaries_records_bool_and_value() {
    // One call records two trace entries: the ratio's bool and the
    // chosen value, in that order, on either branch.
    let result = noprop::Runner::new(5).run(1, |ctx| {
        let _v = noprop::sample_with_boundaries(
            ctx,
            &[0, 1500, u32::MAX],
            noprop::Ratio::one_nth(10),
            noprop::sample_u32,
        );
        panic!("stop");
    });
    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 2, "generated: {generated:?}");
    assert_eq!(generated[0].type_name(), "bool");
    assert_eq!(generated[1].type_name(), "u32");
}

#[test]
fn sample_with_boundaries_is_reproducible_across_runs() {
    // Two independent Runner invocations with the same seed must
    // produce the same failure case index: the boundary hit occurs
    // with probability 1/2, so the case index matters.
    let seed = 0xBAD_CAFE_1234_5678u64;
    let run = || {
        noprop::Runner::new(seed).run(64, |ctx| {
            let v = noprop::sample_with_boundaries(
                ctx,
                &[u32::MAX],
                noprop::Ratio::one_nth(2),
                noprop::sample_u32,
            );
            assert!(v != u32::MAX, "hit boundary");
            Ok(())
        })
    };
    let a = run().expect_err("expected Err");
    let b = run().expect_err("expected Err");
    assert_eq!(a.case_index(), b.case_index());
}

#[test]
fn sample_weighted_index_records_only_the_chosen_index() {
    let result = noprop::Runner::new(5).run(1, |ctx| {
        let _idx = noprop::sample_weighted_index(ctx, &[1, 2, 3, 4]);
        panic!("stop");
    });
    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 1, "generated: {generated:?}");
    assert_eq!(generated[0].type_name(), "usize");
    let repr = generated[0].value_repr().unwrap();
    let idx: usize = repr.parse().unwrap();
    assert!(idx < 4);
}

#[test]
fn selection_primitives_are_reproducible_across_runs() {
    // Two independent Runner invocations with the same seed must
    // produce the same failure case index when the property only calls
    // the new selection primitives.
    let seed = 0xC0FF_EE99_1234_5678u64;
    let run = || {
        noprop::Runner::new(seed).run(64, |ctx| {
            let idx = noprop::sample_weighted_index(ctx, &[1, 1, 1, 1]);
            // `n` participates in the choice sequence so the test
            // still exercises `sample_usize_in`, but is deliberately
            // absent from the failure condition — including it would
            // make P(no failure in 64 cases) too high (~37% under the
            // previous `n < 25` factor), turning the test flaky on any
            // internal RNG change.
            let _n = noprop::sample_usize_in(ctx, 0..=100);
            let flip = noprop::sample_ratio(ctx, noprop::Ratio::one_nth(4));
            // P(fail per case) = 1/4 * 1/4 = 1/16, so P(no failure in
            // 64 cases) = (15/16)^64 ≈ 1.6% - safe headroom against
            // future RNG-stream shifts while still leaving the case
            // index seed-dependent.
            assert!(!(flip && idx == 0), "hit forbidden pattern");
            Ok(())
        })
    };
    let a = run().expect_err("expected Err");
    let b = run().expect_err("expected Err");
    assert_eq!(a.case_index(), b.case_index());
}

// === bounded rejection sampling + iteration rejection ===

#[test]
fn sample_with_rejection_returns_first_accepted_value() -> noprop::TestResult {
    noprop::Runner::new(1).run(8, |ctx| {
        let v = noprop::sample_with_rejection(ctx, 4, |ctx| {
            let x = noprop::sample_u32(ctx);
            x.is_multiple_of(2).then_some(x)
        });
        assert!(v.is_multiple_of(2));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn reject_case_retries_iteration_without_counting_it() -> noprop::TestResult {
    // Reject on the first N cases then succeed forever. All N
    // rejections must not consume the cases budget.
    let attempts = std::cell::Cell::new(0usize);
    let accepted = std::cell::Cell::new(0usize);
    let target_accepts = 4;
    noprop::Runner::new(42).run(target_accepts, |ctx| {
        let n = attempts.get();
        attempts.set(n + 1);
        if n < 3 {
            ctx.reject_case();
        }
        accepted.set(accepted.get() + 1);
        Ok(())
    })?;
    assert_eq!(accepted.get(), target_accepts);
    // 3 rejections + 4 acceptances = 7 total invocations.
    assert_eq!(attempts.get(), 3 + target_accepts);
    Ok(())
}

#[test]
fn always_reject_hits_too_many_rejections_and_reports_case_index_zero() {
    // Runner cannot accept any iteration; TooManyRejections should
    // fire and report case_index = 0 (no accepted iteration).
    let result = noprop::Runner::new(7).run(8, |ctx| {
        ctx.reject_case();
    });
    let err = result.expect_err("expected TooManyRejections");
    assert_eq!(err.seed(), 7);
    assert_eq!(err.case_index(), 0);
    let debug = format!("{err:?}");
    assert!(
        debug.contains("too_many_rejections"),
        "debug output should mention too_many_rejections: {debug}"
    );
    let display = format!("{err}");
    assert!(
        display.contains("too many rejections"),
        "display output should mention too many rejections: {display}"
    );
}

#[test]
fn always_reject_is_reproducible_from_seed() {
    let seed = 0xBEEF_1234u64;
    let run = || {
        noprop::Runner::new(seed).run(4, |ctx| {
            ctx.reject_case();
        })
    };
    let a = run().expect_err("expected Err");
    let b = run().expect_err("expected Err");
    assert_eq!(a.seed(), b.seed());
    assert_eq!(a.case_index(), b.case_index());
}

#[test]
fn too_many_rejections_generated_carries_last_rejected_iteration_trace() {
    // RunError::generated() documents that for TooManyRejections it
    // returns the (discarded) trace of the last rejected iteration -
    // not empty. A property that samples one value and then rejects
    // must hit the cap with generated() carrying that one trace
    // entry.
    let err = noprop::Runner::new(0xB5_B5_B5_B5)
        .run(1, |ctx| {
            let _marker = noprop::sample_u32(ctx);
            ctx.reject_case();
        })
        .expect_err("always-rejecting property must hit the rejection cap");
    assert_eq!(err.kind(), noprop::RunErrorKind::TooManyRejections);
    assert_eq!(
        err.generated().len(),
        1,
        "trace must carry the one sample_u32 entry from the last rejected iteration"
    );
}

#[test]
fn rejection_state_overrides_user_catch_returning_ok() -> noprop::TestResult {
    // User code catches the private marker and returns Ok(()) — the
    // runner must still treat the iteration as rejected.
    let attempts = std::cell::Cell::new(0usize);
    noprop::Runner::new(1).run(2, |ctx| {
        let n = attempts.get();
        attempts.set(n + 1);
        if n == 0 {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                ctx.reject_case();
            }));
            // If rejection state didn't override, this Ok would count.
        }
        Ok(())
    })?;
    // First call: caught marker, but runner still rejected the iteration.
    // Second call: normal Ok — counts as the first accepted iteration.
    // Third call: normal Ok — counts as the second accepted iteration.
    assert_eq!(attempts.get(), 3);
    Ok(())
}

#[test]
fn rejection_state_overrides_user_catch_and_reraise() -> noprop::TestResult {
    // User catches the marker then panics with a different payload —
    // the runner must still treat it as rejection, not property failure.
    let attempts = std::cell::Cell::new(0usize);
    noprop::Runner::new(1).run(1, |ctx| {
        let n = attempts.get();
        attempts.set(n + 1);
        if n == 0 {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                ctx.reject_case();
            }));
            std::panic::panic_any("re-raised, should be dropped");
        }
        Ok(())
    })?;
    assert_eq!(attempts.get(), 2);
    Ok(())
}

#[test]
fn sample_with_rejection_all_rejected_triggers_iteration_rejection() -> noprop::TestResult {
    // A closure that always returns None inside sample_with_rejection
    // exhausts and calls reject_case; the runner retries.
    let outer_attempts = std::cell::Cell::new(0usize);
    noprop::Runner::new(1).run(2, |ctx| {
        let n = outer_attempts.get();
        outer_attempts.set(n + 1);
        if n < 2 {
            // First two invocations reject via sample_with_rejection exhaustion.
            let _: u32 = noprop::sample_with_rejection(ctx, 4, |_ctx| None);
            unreachable!("sample_with_rejection exhaustion should unwind");
        }
        Ok(())
    })?;
    // Two cases rejected + two accepted = 4 outer invocations.
    assert_eq!(outer_attempts.get(), 4);
    Ok(())
}

// === sample_string / sample_ascii_string / sample_ascii_printable_string trace format ===

#[test]
fn sample_string_records_one_entry_per_call() {
    let result = noprop::Runner::new(11).run(1, |ctx| {
        let a = noprop::sample_string(ctx, 4);
        let b = noprop::sample_ascii_string(ctx, 4);
        let c = noprop::sample_ascii_printable_string(ctx, 4);
        panic!("stop with a={a:?} b={b:?} c={c:?}");
    });
    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 3, "generated: {generated:?}");
    for entry in generated {
        assert_eq!(entry.type_name(), "alloc::string::String");
        assert!(entry.location().file().ends_with("e2e.rs"));
        let repr = entry.value_repr().expect("value entry");
        // Debug output for a String starts and ends with a double quote.
        assert!(
            repr.starts_with('"') && repr.ends_with('"'),
            "expected quoted Debug form, got {repr:?}"
        );
    }
}

// === sample_f32 / sample_f64 trace format ===

#[test]
fn sample_floats_record_type_and_value() {
    let result = noprop::Runner::new(5).run(1, |ctx| {
        let a = noprop::sample_f32(ctx);
        let b = noprop::sample_f64(ctx);
        panic!("stop with a={a} b={b}");
    });
    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 2, "generated: {generated:?}");
    assert_eq!(generated[0].type_name(), "f32");
    assert_eq!(generated[1].type_name(), "f64");
    let a_repr: f32 = generated[0].value_repr().unwrap().parse().unwrap();
    let b_repr: f64 = generated[1].value_repr().unwrap().parse().unwrap();
    assert!(a_repr.is_finite());
    assert!(b_repr.is_finite());
    assert!(generated[0].location().file().ends_with("e2e.rs"));
}

// === reproduce-hint line in failure Display / Debug ===

#[test]
fn failure_display_contains_reproduce_line_that_reproduces_the_same_failure() {
    // Force a failure whose case index is not zero, so `cases`
    // and `case_index + 1` are meaningfully distinct.
    let seed = 0x5EED_1EAD_BEEF_C0DEu64;
    let target = std::cell::Cell::new(0usize);
    let run = || {
        target.set(0);
        noprop::Runner::new(seed).run(128, |_ctx| {
            let n = target.get();
            target.set(n + 1);
            if n >= 3 {
                panic!("boom at iteration {n}");
            }
            Ok(())
        })
    };

    let err = run().expect_err("expected panic to become Err");
    assert_eq!(err.case_index(), 3);

    let display = format!("{err}");
    // The hint reuses the original case budget so the rerun hits
    // the same rejection cap (a `case_index + 1` hint would shrink it).
    let expected_cases = 128;
    let hint = format!(
        "reproduce with: noprop::Runner::new({:#018x}).run({expected_cases}, |ctx| ...)",
        err.seed(),
    );
    assert!(
        display.contains(&hint),
        "Display should contain reproduce hint {hint:?}, got:\n{display}"
    );

    // Debug output uses a slightly different framing but must carry the
    // same seed and cases.
    let debug = format!("{err:?}");
    let debug_hint = format!(
        "reproduce: noprop::Runner::new({:#018x}).run({expected_cases}, |ctx| ...)",
        err.seed(),
    );
    assert!(
        debug.contains(&debug_hint),
        "Debug should contain reproduce hint {debug_hint:?}, got:\n{debug}"
    );

    // Using the hint verbatim should reproduce the same failure.
    let target = std::cell::Cell::new(0usize);
    let replay = noprop::Runner::new(err.seed()).run(expected_cases, |_ctx| {
        let n = target.get();
        target.set(n + 1);
        if n >= 3 {
            panic!("boom at iteration {n}");
        }
        Ok(())
    });
    let replayed = replay.expect_err("hint cases must reproduce the failure");
    assert_eq!(replayed.seed(), err.seed());
    assert_eq!(replayed.case_index(), err.case_index());
}

// === Stats ===

#[test]
fn stats_success_reports_accepted_cases_and_zero_rejections() -> noprop::TestResult {
    let mut runner = noprop::Runner::new(0xDEAD_BEEF);
    runner.run(10, |ctx| {
        // Two sample_* per case => total_samples = 2 * cases for a
        // clean run.
        let _a = noprop::sample_u32(ctx);
        let _b = noprop::sample_u32(ctx);
        Ok(())
    })?;
    let stats = runner.stats();
    assert_eq!(stats.accepted_cases, 10);
    assert_eq!(stats.rejected_cases, 0);
    assert_eq!(stats.total_samples, 20);
    Ok(())
}

#[test]
fn stats_counts_reject_case_unwinds() -> noprop::TestResult {
    use std::cell::Cell;
    let counter = Cell::new(0usize);
    let mut runner = noprop::Runner::new(1);
    runner.run(3, |ctx| {
        let n = counter.get();
        counter.set(n + 1);
        // First two invocations reject, then every subsequent one accepts.
        if n < 2 {
            ctx.reject_case();
        }
        Ok(())
    })?;
    let stats = runner.stats();
    assert_eq!(stats.accepted_cases, 3);
    assert_eq!(stats.rejected_cases, 2);
    Ok(())
}

#[test]
fn stats_counts_sample_with_rejection_exhaustion_as_rejected_iteration() {
    let mut runner = noprop::Runner::new(1);
    let result = runner.run(1, |ctx| {
        // Attempt closure only accepts u == 0 (~2⁻³² per attempt), so
        // every sample_with_rejection call exhausts and unwinds the
        // iteration via reject_case. The runner will eventually give up
        // with TooManyRejections.
        let _x = noprop::sample_with_rejection(ctx, 4, |ctx| {
            let u = noprop::sample_u32(ctx);
            (u == 0).then_some(u)
        });
        Ok(())
    });
    let err = result.expect_err("all-reject property must fail");
    // Same Stats value should be reachable from both err.stats() and
    // runner.stats().
    assert_eq!(err.stats(), runner.stats());
    let s = runner.stats();
    assert_eq!(s.accepted_cases, 0);
    assert!(
        s.rejected_cases > 0,
        "expected some rejected cases, got {}",
        s.rejected_cases
    );
}

#[test]
fn stats_is_deterministic_per_seed() -> noprop::TestResult {
    let mut a = noprop::Runner::new(42);
    a.run(5, |ctx| {
        let _ = noprop::sample_u32(ctx);
        let _ = noprop::sample_bool(ctx);
        Ok(())
    })?;
    let mut b = noprop::Runner::new(42);
    b.run(5, |ctx| {
        let _ = noprop::sample_u32(ctx);
        let _ = noprop::sample_bool(ctx);
        Ok(())
    })?;
    assert_eq!(a.stats(), b.stats());
    Ok(())
}

#[test]
fn stats_on_failure_reports_progress_up_to_failing_case() {
    use std::cell::Cell;
    let counter = Cell::new(0usize);
    let err = noprop::Runner::new(7)
        .run(10, |_ctx| {
            let n = counter.get();
            counter.set(n + 1);
            if n == 4 {
                panic!("boom at iteration {n}");
            }
            Ok(())
        })
        .expect_err("closure must fail at case 4");
    let stats = err.stats();
    // Four cases passed before the panic on the fifth (index 4).
    assert_eq!(stats.accepted_cases, 4);
    assert_eq!(stats.rejected_cases, 0);
    assert_eq!(err.case_index(), stats.accepted_cases);
}

// === Feedback-guided PBT (Runner::run_feedback_guided / event / bucket / transition) ===

#[test]
fn run_feedback_guided_succeeds_with_features() {
    let mut runner = noprop::Runner::new(42);
    runner
        .run_feedback_guided(64, |ctx| {
            let x = noprop::sample_u32(ctx);
            if x.is_multiple_of(4) {
                ctx.event("multiple-of-four");
            }
            Ok(())
        })
        .expect("feedback-guided run with semantic feedback must succeed");
    let stats = runner.stats();
    assert_eq!(stats.accepted_cases, 64);
}

#[test]
fn run_feedback_guided_without_features_succeeds() {
    // Feedback-guided mode does not require feedback: a property that
    // never reports a feature just yields no interesting cases.
    let mut runner = noprop::Runner::new(1);
    runner
        .run_feedback_guided(8, |_ctx| Ok(()))
        .expect("a property without semantic feedback must not fail");
    let stats = runner.stats();
    assert_eq!(stats.accepted_cases, 8);
    assert_eq!(stats.rejected_cases, 0);
}

#[test]
fn feedback_reported_from_the_first_case_drives_the_search() {
    // Every case reports a feature, so the corpus is non-empty from
    // the first case: exploratory candidates replay the admitted
    // draws, and the run stays deterministic per seed.
    use std::cell::Cell;
    fn run(seed: u64) -> Vec<u32> {
        let observed: Cell<Vec<u32>> = Cell::new(Vec::new());
        noprop::Runner::new(seed)
            .run_feedback_guided(64, |ctx| {
                let x = noprop::sample_u32(ctx);
                let mut v = observed.take();
                v.push(x);
                observed.set(v);
                ctx.event("always");
                Ok(())
            })
            .expect("run must succeed");
        observed.into_inner()
    }
    let a = run(42);
    let b = run(42);
    assert_eq!(a, b, "the candidate stream must be reproducible");
}

#[test]
fn feedback_reported_late_is_captured_from_the_case_start() {
    // The property reports a feature only after several draws: the
    // feedback-guided runner records the whole case from its start, so
    // the draws made before the report are part of the admitted
    // sequence and the run must not fail or lose determinism.
    use std::cell::Cell;
    fn run(seed: u64) -> Vec<usize> {
        let observed: Cell<Vec<usize>> = Cell::new(Vec::new());
        noprop::Runner::new(seed)
            .run_feedback_guided(64, |ctx| {
                let mut low = 0usize;
                let mut high = 0usize;
                for _ in 0..4 {
                    let x = noprop::sample_u32(ctx);
                    if x < u32::MAX / 2 {
                        low += 1;
                    } else {
                        high += 1;
                    }
                }
                if high > low {
                    ctx.event("high-dominant");
                }
                let mut v = observed.take();
                v.push(high);
                observed.set(v);
                Ok(())
            })
            .expect("run must succeed");
        observed.into_inner()
    }
    let seed = 0xC0FFEE;
    assert_eq!(run(seed), run(seed), "late feedback must stay reproducible");
    assert!(
        run(seed).iter().any(|&high| high > 2),
        "the feature region must be explored: {:?}",
        run(seed)
    );
}

#[test]
fn feedback_never_reported_is_a_valid_run() {
    // A property that never calls a feedback method is a valid
    // feedback-guided run: it just yields no interesting cases.
    let mut runner = noprop::Runner::new(7);
    runner
        .run_feedback_guided(32, |ctx| {
            let _ = noprop::sample_u32(ctx);
            Ok(())
        })
        .expect("a run without feedback must not fail");
    let stats = runner.stats();
    assert_eq!(stats.accepted_cases, 32);
    assert_eq!(stats.discovered_features, 0);
    assert_eq!(stats.max_corpus_size, 0);
}

#[test]
fn run_feedback_guided_zero_cases_does_not_invoke_closure() {
    let invoked = std::cell::Cell::new(0usize);
    let mut runner = noprop::Runner::new(1);
    runner
        .run_feedback_guided(0, |ctx| {
            invoked.set(invoked.get() + 1);
            ctx.event("e");
            Ok(())
        })
        .expect("zero cases must succeed");
    assert_eq!(invoked.get(), 0, "the closure must not be invoked");
    assert_eq!(runner.stats(), noprop::Stats::default());
}

#[test]
fn semantic_methods_are_noop_under_plain_run() {
    noprop::Runner::new(1)
        .run(8, |ctx| {
            ctx.event("e");
            ctx.bucket("b", 1);
            ctx.transition("t", 0, 1);
            Ok(())
        })
        .expect("semantic methods must be ignored by the plain runner");
}

#[test]
fn run_feedback_guided_reports_property_failure() {
    let err = noprop::Runner::new(1)
        .run_feedback_guided(32, |ctx| {
            ctx.event("before-failure");
            panic!("deterministic failure");
        })
        .expect_err("panicking closure must fail the run");
    let display = format!("{err}");
    assert!(display.contains("deterministic failure"), "{display}");
    assert!(
        display.contains("run_feedback_guided(32, |ctx| ...)"),
        "{display}"
    );
    assert!(display.contains("Semantic features:"), "{display}");
    assert!(display.contains("event(\"before-failure\")"), "{display}");
    assert_eq!(err.case_index(), 0);
}

#[test]
fn candidate_index_accessor_matches_semantics() {
    // The public accessor returns Some(candidate_ordinal) for
    // feedback-guided failures and None for uniform failures. This
    // guards the accessor contract independent of the Debug format
    // (existing candidate_index tests scrape Debug output).
    let uniform_err = noprop::Runner::new(1)
        .run(4, |_ctx| panic!("uniform fail"))
        .expect_err("uniform run must fail");
    assert_eq!(
        uniform_err.candidate_index(),
        None,
        "Runner::run failures do not carry a candidate index"
    );

    let feedback_err = noprop::Runner::new(1)
        .run_feedback_guided(4, |ctx| {
            ctx.event("e");
            panic!("feedback fail on first attempt");
        })
        .expect_err("feedback-guided run must fail");
    assert_eq!(
        feedback_err.candidate_index(),
        Some(1),
        "the first attempt is candidate 1 (one-based)"
    );
}

#[test]
fn run_feedback_guided_candidate_index_is_one_based() {
    // The candidate index counts every attempt (accepted, rejected,
    // and the failing case itself) and is one-based, unlike the
    // zero-based accepted-iteration `case_index`.
    let attempts = std::cell::Cell::new(0usize);
    let err = noprop::Runner::new(1)
        .run_feedback_guided(8, |ctx| {
            attempts.set(attempts.get() + 1);
            ctx.event("e");
            panic!("fail on first attempt");
        })
        .expect_err("first attempt must fail");
    let debug = format!("{err:?}");
    assert!(
        debug.contains("candidate_index: 1"),
        "the first attempt must be candidate 1: {debug}"
    );
    assert!(debug.contains("case_index: 0"), "{debug}");
    assert_eq!(attempts.get(), 1);
}

#[test]
fn run_feedback_guided_candidate_index_counts_rejected_attempts() {
    // A rejected attempt still advances the candidate index, so the
    // failing second attempt is candidate 2 while `case_index` stays 0
    // (no accepted iteration ran).
    let attempts = std::cell::Cell::new(0usize);
    let err = noprop::Runner::new(7)
        .run_feedback_guided(8, |ctx| {
            let n = attempts.get();
            attempts.set(n + 1);
            ctx.event("e");
            if n == 0 {
                ctx.reject_case();
            }
            panic!("fail on second attempt");
        })
        .expect_err("second attempt must fail");
    let debug = format!("{err:?}");
    assert!(
        debug.contains("candidate_index: 2"),
        "the rejected attempt must count toward the candidate index: {debug}"
    );
    assert!(debug.contains("case_index: 0"), "{debug}");
    assert_eq!(err.stats().rejected_cases, 1);
}

#[test]
fn run_feedback_guided_reports_err_closure_failure_with_semantics() {
    let err = noprop::Runner::new(1)
        .run_feedback_guided(8, |ctx| {
            ctx.event("before-error");
            Err("corpus error".into())
        })
        .expect_err("returned Err must fail the run");
    let display = format!("{err}");
    assert!(display.contains("corpus error"), "{display}");
    assert!(
        display.contains("run_feedback_guided(8, |ctx| ...)"),
        "{display}"
    );
    assert!(display.contains("Semantic features:"), "{display}");
    assert!(display.contains("event(\"before-error\")"), "{display}");
    assert_eq!(err.case_index(), 0);
    assert!(format!("{err:?}").contains("candidate_index: 1"), "{err:?}");
}

#[test]
fn run_feedback_guided_counts_rejections() {
    let mut runner = noprop::Runner::new(3);
    runner
        .run_feedback_guided(16, |ctx| {
            let x = noprop::sample_u32(ctx);
            if x.is_multiple_of(2) {
                ctx.reject_case();
            }
            ctx.event("accepted");
            Ok(())
        })
        .expect("rejections must be retried like the plain runner");
    let stats = runner.stats();
    assert_eq!(stats.accepted_cases, 16);
    assert!(stats.rejected_cases > 0);
}

#[test]
fn run_feedback_guided_too_many_rejections_reports_last_rejected_semantics() {
    // The rejection-cap failure must carry the semantic features of
    // the last rejected case and its candidate index (every attempt
    // was rejected, so the index equals the rejected count). This
    // guards against the report silently dropping to "candidate_index:
    // 0" on this path.
    let err = noprop::Runner::new(1)
        .run_feedback_guided(8, |ctx| {
            ctx.event("always-reject");
            ctx.reject_case();
        })
        .expect_err("always-rejecting must hit the rejection cap");
    let debug = format!("{err:?}");
    assert!(debug.contains("too_many_rejections"), "{debug}");
    assert!(
        debug.contains("event(\"always-reject\")"),
        "the last rejected case's features must be reported: {debug}"
    );
    // All attempts were rejected, so the candidate index of the last
    // rejected attempt equals the rejected count. Avoid hard-coding
    // the cap formula (it is deliberately crate-private).
    assert!(
        debug.contains(&format!("candidate_index: {}", err.stats().rejected_cases)),
        "candidate_index must equal the rejected count: {debug}"
    );
    let display = format!("{err}");
    assert!(display.contains("too many rejections"), "{display}");
    assert!(
        display.contains("run_feedback_guided(8, |ctx| ...)"),
        "{display}"
    );
    assert!(display.contains("Semantic features:"), "{display}");
}

#[test]
fn feedback_guided_tmr_error_pins_hint_stats_and_features() {
    // The too-many-rejections report must carry the feedback-guided
    // reproduce hint (so the rerun reproduces the same exit), the
    // corpus stats fields, and the last rejected case's semantic
    // features. The existing tests only substring-match the hint; this
    // pins the full hint and the corpus fields.
    let err = noprop::Runner::new(7)
        .run_feedback_guided(8, |ctx| {
            ctx.event("always-reject");
            ctx.reject_case();
        })
        .expect_err("always-rejecting must hit the rejection cap");
    let display = format!("{err}");
    assert!(
        display.contains(&format!(
            "reproduce with: noprop::Runner::new({:#018x}).run_feedback_guided(8, |ctx| ...)",
            err.seed()
        )),
        "the hint must name the feedback-guided entry point: {display}"
    );
    assert!(display.contains("Semantic features:"), "{display}");
    assert!(display.contains("event(\"always-reject\")"), "{display}");
    // Every rejected case reports the same feature, so exactly one
    // feature is observed and exactly one entry is admitted.
    let stats = err.stats();
    assert_eq!(stats.discovered_features, 1);
    assert_eq!(stats.max_corpus_size, 1);
    let debug = format!("{err:?}");
    assert!(
        debug.contains(&format!(
            "stats: {{ accepted: 0, rejected: {}, total_samples: 0, discovered_features: 1, max_corpus_size: 1 }},",
            stats.rejected_cases
        )),
        "the Debug stats line must include the corpus fields: {debug}"
    );
}

#[test]
fn run_feedback_guided_bounds_high_cardinality_features() {
    // A property that reports effectively unbounded bucket values must
    // not crash or grow memory without bound. This is a smoke test:
    // with 64 cases × 3 buckets it stays far below the per-case
    // (64) and global (1024) caps, so neither cap nor eviction fires
    // here — those bounds are exercised by the SemanticCorpus unit
    // tests. The run succeeding regardless of the reported values is
    // the point.
    let mut runner = noprop::Runner::new(5);
    runner
        .run_feedback_guided(64, |ctx| {
            let x = noprop::sample_u64(ctx);
            ctx.bucket("unbounded", x);
            ctx.bucket("also-unbounded", x.wrapping_add(1));
            ctx.bucket("yet-another", x.wrapping_mul(3));
            Ok(())
        })
        .expect("high-cardinality features must be capped, not fatal");
    let stats = runner.stats();
    assert_eq!(stats.accepted_cases, 64);
}

#[test]
fn run_feedback_guided_rejected_cases_register_features() {
    // A rejected case that reported a novel feature before rejecting
    // must be admitted into the rejected queue (low-energy
    // scaffolding), so its features count toward the global registry.
    // This is observable through the run's behaviour: the same feature
    // reported later by an accepted case is no longer novel.
    let mut runner = noprop::Runner::new(11);
    runner
        .run_feedback_guided(32, |ctx| {
            let x = noprop::sample_u32(ctx);
            ctx.event("shared-feature");
            if x.is_multiple_of(2) {
                ctx.reject_case();
            }
            Ok(())
        })
        .expect("rejected cases with novel features must be tolerated");
    let stats = runner.stats();
    assert_eq!(stats.accepted_cases, 32);
    assert!(stats.rejected_cases > 0);
}

#[test]
fn run_feedback_guided_is_reproducible_from_seed() {
    let seed = 0xABCD_EF01_2345_6789;

    fn run(seed: u64) -> Vec<u32> {
        let observed = std::cell::Cell::new(Vec::new());
        noprop::Runner::new(seed)
            .run_feedback_guided(64, |ctx| {
                let x = noprop::sample_u32(ctx);
                let mut v = observed.take();
                v.push(x);
                observed.set(v);
                if x.is_multiple_of(2) {
                    ctx.event("even");
                }
                Ok(())
            })
            .expect("feedback-guided run must succeed");
        observed.into_inner()
    }

    assert_eq!(run(seed), run(seed));
}

#[test]
fn run_feedback_guided_with_rejections_is_reproducible_from_seed() {
    // The plain reproducibility test never rejects, so the
    // rejected-queue pick path (and the PRNG rolls it consumes) stays
    // unexercised. This property rejects a fraction of candidates so
    // the full candidate stream — including the rejected-queue branch
    // of `next_context` — must reproduce from the seed.
    use std::cell::Cell;

    fn run(seed: u64) -> Vec<u32> {
        let observed: Cell<Vec<u32>> = Cell::new(Vec::new());
        noprop::Runner::new(seed)
            .run_feedback_guided(64, |ctx| {
                let x = noprop::sample_u32(ctx);
                let mut v = observed.take();
                v.push(x);
                observed.set(v);
                if x.is_multiple_of(2) {
                    ctx.event("even");
                }
                if x.is_multiple_of(4) {
                    ctx.reject_case();
                }
                Ok(())
            })
            .expect("feedback-guided run must succeed");
        observed.into_inner()
    }

    let seed = 0xC0FFEE;
    let a = run(seed);
    let b = run(seed);
    assert_eq!(a, b, "rejected candidates must reproduce from the seed");
    assert!(
        a.iter().any(|x| x.is_multiple_of(4)),
        "the rejected path must actually run: {a:?}"
    );
}

#[test]
fn run_feedback_guided_stats_count_rejected_attempt_samples() {
    // `total_samples` must include samples produced by rejected
    // attempts (they consumed generator budget), and
    // `rejected_cases` must count every `reject_case`.
    use std::cell::Cell;

    let attempts = Cell::new(0usize);
    let mut runner = noprop::Runner::new(1);
    runner
        .run_feedback_guided(4, |ctx| {
            attempts.set(attempts.get() + 1);
            ctx.event("e");
            let _ = noprop::sample_u32(ctx);
            if attempts.get() == 1 {
                ctx.reject_case();
            }
            Ok(())
        })
        .expect("run must succeed");
    let stats = runner.stats();
    assert_eq!(stats.accepted_cases, 4);
    assert_eq!(stats.rejected_cases, 1);
    assert_eq!(
        stats.total_samples, 5,
        "1 rejected + 4 accepted attempts, one sample each"
    );
}

#[test]
fn run_feedback_guided_replays_rejected_candidates() {
    // The rejected queue is a mutation parent: an early rejected case
    // with a novel feature enters the rejected queue and — while the
    // accepted queue stays empty — it is the only source for
    // exploratory candidates. Replaying it keeps the observed values in
    // the rejected region (a uniform-only run would have a median
    // around 500).
    use std::cell::Cell;

    let observed: Cell<Vec<usize>> = Cell::new(Vec::new());
    noprop::Runner::new(1)
        .run_feedback_guided(256, |ctx| {
            let x = noprop::sample_usize_in(ctx, 0..1000);
            let mut v = observed.take();
            v.push(x);
            observed.set(v);
            if x < 100 {
                ctx.event("low");
                ctx.reject_case();
            }
            Ok(())
        })
        .expect("run must succeed");
    let mut sorted = observed.into_inner();
    sorted.sort_unstable();
    assert!(
        sorted[sorted.len() / 2] < 200,
        "the rejected entry must be replayed as a mutation parent: median {}",
        sorted[sorted.len() / 2]
    );
}

#[test]
fn run_feedback_guided_steers_candidates_by_novel_features() {
    // The corpus must concentrate the search on the interesting input
    // region: the first case that reaches the "high" feature is
    // admitted and replayed (unmutated with probability 3/4) as the
    // mutation parent, so the second-half median of observed values
    // sits inside the feature region. A uniform-only run would have a
    // median around 500, so a max comparison would pass on restart
    // noise alone; the median is asserted instead.
    use std::cell::Cell;

    let observed: Cell<Vec<usize>> = Cell::new(Vec::new());
    noprop::Runner::new(7)
        .run_feedback_guided(256, |ctx| {
            let x = noprop::sample_usize_in(ctx, 0..1000);
            let mut v = observed.take();
            v.push(x);
            observed.set(v);
            if x > 900 {
                ctx.event("high");
            }
            Ok(())
        })
        .expect("feedback-guided run must succeed");
    let mut second_half: Vec<usize> = observed.into_inner()[128..].to_vec();
    second_half.sort_unstable();
    let second_half_median = second_half[second_half.len() / 2];
    assert!(
        second_half_median > 900,
        "the corpus must concentrate the search on the feature region: \
         second-half median reached {second_half_median}"
    );
}

#[test]
fn run_feedback_guided_steers_stateful_transitions() {
    // A stateful-style target: the property advances an abstract state
    // machine and reports each transition. The corpus must explore
    // deeper transition chains than uniform sampling. The mean depth of
    // the second half is asserted (the max is a single tail point and
    // would pass on uniform-restart noise alone).
    use std::cell::Cell;

    fn observe(seed: u64, corpus_guided: bool) -> Vec<usize> {
        let observed: Cell<Vec<usize>> = Cell::new(Vec::new());
        let mut runner = noprop::Runner::new(seed);
        let property = |ctx: &mut noprop::TestCaseContext| {
            let mut state = 0u64;
            for _ in 0..64 {
                let step = noprop::sample_usize_in(ctx, 0..2);
                if step != 0 {
                    break;
                }
                let next = state + 1;
                ctx.transition("advance", state, next);
                state = next;
            }
            let mut v = observed.take();
            v.push(state as usize);
            observed.set(v);
            Ok(())
        };
        if corpus_guided {
            runner
                .run_feedback_guided(256, property)
                .expect("feedback-guided run must succeed");
        } else {
            runner.run(256, property).expect("uniform run must succeed");
        }
        observed.into_inner()
    }

    let mean = |xs: &[usize]| xs[128..].iter().sum::<usize>() as f64 / 128.0;

    let uniform_depths = observe(9, false);
    let corpus_depths = observe(9, true);
    let uniform_mean = mean(&uniform_depths);
    let corpus_mean = mean(&corpus_depths);
    assert!(
        corpus_mean > uniform_mean + 0.5,
        "the corpus must explore deeper transition chains than uniform sampling: \
         corpus mean depth {corpus_mean}, uniform {uniform_mean}"
    );
}

#[test]
fn run_feedback_guided_reports_corpus_fields() {
    // The corpus fields are non-zero for feedback-guided runs. The exact
    // values and the caps (feature set 1024, corpus 64) are verified
    // in unit tests; e2e only checks the 0 / non-zero distinction.
    let case = std::cell::Cell::new(0u64);
    let mut runner = noprop::Runner::new(1);
    runner
        .run_feedback_guided(4, |ctx| {
            let i = case.get();
            case.set(i + 1);
            ctx.bucket("b", i);
            Ok(())
        })
        .expect("feedback-guided run must succeed");
    let stats = runner.stats();
    assert!(stats.discovered_features > 0);
    assert!(stats.max_corpus_size > 0);
}

#[test]
fn run_feedback_guided_failure_error_carries_corpus_stats() {
    // A property failure must embed the corpus stats in the error. The
    // failing case's feature is not admitted, so on a first-case
    // failure the corpus fields are 0 (see `Stats` docs).
    let err = noprop::Runner::new(1)
        .run_feedback_guided(32, |ctx| {
            ctx.event("before-failure");
            panic!("deterministic failure");
        })
        .expect_err("panicking closure must fail the run");
    let stats = err.stats();
    assert_eq!(stats.accepted_cases, 0);
    assert_eq!(
        stats.discovered_features, 0,
        "failing case features are not counted"
    );
    assert_eq!(stats.max_corpus_size, 0);

    // With accepted cases before the failure, their features are
    // counted.
    let case = std::cell::Cell::new(0u64);
    let err = noprop::Runner::new(1)
        .run_feedback_guided(32, |ctx| {
            let i = case.get();
            case.set(i + 1);
            ctx.bucket("b", i);
            if i >= 4 {
                panic!("fail after four accepted cases");
            }
            Ok(())
        })
        .expect_err("panicking closure must fail the run");
    let stats = err.stats();
    // The four accepted cases each register a novel feature; the
    // failing case (i = 4) is not admitted, so the count is exactly 4.
    assert_eq!(
        stats.discovered_features, 4,
        "accepted cases must be counted"
    );
    assert_eq!(stats.max_corpus_size, 4);
}

#[test]
fn feedback_guided_failure_hint_names_the_entry_point() {
    // A feedback-guided failure must carry a reproduce hint that names
    // `run_feedback_guided`, so the rerun reproduces the failure.
    let err = noprop::Runner::new(1)
        .run_feedback_guided(8, |ctx| {
            ctx.event("e");
            panic!("deterministic failure");
        })
        .expect_err("panicking closure must fail the run");
    let display = format!("{err}");
    assert!(
        display.contains("run_feedback_guided(8, |ctx| ...)"),
        "Display must name the feedback-guided entry point in the hint, got:\n{display}"
    );
}
