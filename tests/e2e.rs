//! End-to-end integration tests for the noprop runner.
//!
//! These tests exercise the public API as an external user would:
//! everything is referenced via `noprop::` full qualification, with no
//! `use noprop::*` shortcuts, matching the convention documented for
//! all noprop example code.

#[test]
fn run_returns_ok_when_property_holds() -> noprop::Result<()> {
    noprop::Runner::new(0xDEAD_BEEF, 16).run(|ctx| {
        let x = noprop::sample_u32(ctx);
        assert_eq!(x, x);
        Ok(())
    })?;
    Ok(())
}

#[test]
fn run_returns_err_on_failed_assertion() {
    // Property "every u32 is zero" fails almost immediately.
    let result = noprop::Runner::new(0x1234, 64).run(|ctx| {
        let x = noprop::sample_u32(ctx);
        assert_eq!(x, 0, "expected zero, got {x}");
        Ok(())
    });

    let err = result.expect_err("expected Err, got Ok");
    assert_eq!(err.seed(), 0x1234);
    assert!(err.case_index() < 64);
}

#[test]
fn same_seed_reproduces_same_failure() {
    let seed = 0xABCD_1234_5678_9ABC;

    let run = || {
        noprop::Runner::new(seed, 32).run(|ctx| {
            let x = noprop::sample_u32(ctx);
            // Roughly half of iterations fail — enough to guarantee an
            // Err within 32 iterations with vanishing probability of Ok.
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
fn zero_iterations_returns_ok_without_invoking_property() -> noprop::Result<()> {
    let invoked = std::cell::Cell::new(false);
    noprop::Runner::new(0, 0).run(|_ctx| {
        invoked.set(true);
        Ok(())
    })?;
    assert!(
        !invoked.get(),
        "property should not be invoked when iterations is 0"
    );
    Ok(())
}

#[test]
fn error_debug_output_contains_seed_and_case() {
    let seed = 0xFEED_FACE_C0DE_BABE;
    let result = noprop::Runner::new(seed, 1).run(|_ctx| {
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
    // Count iterations via a Cell so that the property closure stays a
    // pure `Fn`. Panic on the third invocation and verify the runner
    // stopped there (no fourth invocation).
    let count = std::cell::Cell::new(0usize);
    let _ = noprop::Runner::new(0, 100).run(|_ctx| {
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
    let result = noprop::Runner::new(42, 1).run(|ctx| {
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
    let result = noprop::Runner::new(1, 1).run(|ctx| {
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
    let result = noprop::Runner::new(1, 1).run(|ctx| {
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
    let result = noprop::Runner::new(1, 1).run(|ctx| {
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
    let result = noprop::Runner::new(7, 5).run(|ctx| {
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
    let result = noprop::Runner::new(42, 1).run(|ctx| {
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
    let result = noprop::Runner::new(5, 1).run(|ctx| {
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
    let result = noprop::Runner::new(5, 1).run(|ctx| {
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
    let result = noprop::Runner::new(42, 1).run(|ctx| {
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
    let result = noprop::Runner::new(5, 1).run(|ctx| {
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
    let result = noprop::Runner::new(5, 1).run(|ctx| {
        let _b = noprop::sample_ratio(ctx, 1, 3);
        panic!("stop");
    });
    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 1, "generated: {generated:?}");
    assert_eq!(generated[0].type_name(), "bool");
}

#[test]
fn sample_weighted_index_records_only_the_chosen_index() {
    let result = noprop::Runner::new(5, 1).run(|ctx| {
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
        noprop::Runner::new(seed, 64).run(|ctx| {
            let idx = noprop::sample_weighted_index(ctx, &[1, 1, 1, 1]);
            let n = noprop::sample_usize_in(ctx, 0..=100);
            let flip = noprop::sample_ratio(ctx, 1, 4);
            // Fail on a pattern that is common enough to hit within 64
            // iterations but does not always fire, so the case index
            // matters.
            assert!(!(flip && idx == 0 && n < 25), "hit forbidden pattern");
            Ok(())
        })
    };
    let a = run().expect_err("expected Err");
    let b = run().expect_err("expected Err");
    assert_eq!(a.case_index(), b.case_index());
}

// === bounded rejection sampling + iteration rejection ===

#[test]
fn sample_with_rejection_returns_first_accepted_value() -> noprop::Result<()> {
    noprop::Runner::new(1, 8).run(|ctx| {
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
fn reject_case_retries_iteration_without_counting_it() -> noprop::Result<()> {
    // Reject on the first N iterations then succeed forever. All N
    // rejections must not consume the iterations budget.
    let attempts = std::cell::Cell::new(0usize);
    let accepted = std::cell::Cell::new(0usize);
    let target_accepts = 4;
    noprop::Runner::new(42, target_accepts).run(|ctx| {
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
    let result = noprop::Runner::new(7, 8).run(|ctx| {
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
        noprop::Runner::new(seed, 4).run(|ctx| {
            ctx.reject_case();
        })
    };
    let a = run().expect_err("expected Err");
    let b = run().expect_err("expected Err");
    assert_eq!(a.seed(), b.seed());
    assert_eq!(a.case_index(), b.case_index());
}

#[test]
fn rejection_state_overrides_user_catch_returning_ok() -> noprop::Result<()> {
    // User code catches the private marker and returns Ok(()) — the
    // runner must still treat the iteration as rejected.
    let attempts = std::cell::Cell::new(0usize);
    noprop::Runner::new(1, 2).run(|ctx| {
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
fn rejection_state_overrides_user_catch_and_reraise() -> noprop::Result<()> {
    // User catches the marker then panics with a different payload —
    // the runner must still treat it as rejection, not property failure.
    let attempts = std::cell::Cell::new(0usize);
    noprop::Runner::new(1, 1).run(|ctx| {
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
fn sample_with_rejection_all_rejected_triggers_iteration_rejection() -> noprop::Result<()> {
    // A closure that always returns None inside sample_with_rejection
    // exhausts and calls reject_case; the runner retries.
    let outer_attempts = std::cell::Cell::new(0usize);
    noprop::Runner::new(1, 2).run(|ctx| {
        let n = outer_attempts.get();
        outer_attempts.set(n + 1);
        if n < 2 {
            // First two invocations reject via sample_with_rejection exhaustion.
            let _: u32 = noprop::sample_with_rejection(ctx, 4, |_ctx| None);
            unreachable!("sample_with_rejection exhaustion should unwind");
        }
        Ok(())
    })?;
    // Two iterations rejected + two accepted = 4 outer invocations.
    assert_eq!(outer_attempts.get(), 4);
    Ok(())
}

#[test]
#[should_panic(expected = "Runner::run")]
fn reject_case_outside_runner_panics() {
    let mut ctx = noprop::TestCaseContext::new(0);
    ctx.reject_case();
}

// === sample_string / sample_ascii_string / sample_ascii_printable_string trace format ===

#[test]
fn sample_string_records_one_entry_per_call() {
    let result = noprop::Runner::new(11, 1).run(|ctx| {
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

// === sample_f32_finite / sample_f64_finite trace format ===

#[test]
fn sample_finite_floats_record_type_and_value() {
    let result = noprop::Runner::new(5, 1).run(|ctx| {
        let a = noprop::sample_f32_finite(ctx);
        let b = noprop::sample_f64_finite(ctx);
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
    // Force a failure whose case index is not zero, so `iterations`
    // and `case_index + 1` are meaningfully distinct.
    let seed = 0x5EED_1EAD_BEEF_C0DEu64;
    let target = std::cell::Cell::new(0usize);
    let run = || {
        target.set(0);
        noprop::Runner::new(seed, 128).run(|_ctx| {
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
    // The hint reuses the original iteration budget so the rerun hits
    // the same rejection cap (a `case_index + 1` hint would shrink it).
    let expected_iterations = 128;
    let hint = format!(
        "reproduce with: noprop::Runner::new({:#018x}, {expected_iterations})",
        err.seed(),
    );
    assert!(
        display.contains(&hint),
        "Display should contain reproduce hint {hint:?}, got:\n{display}"
    );

    // Debug output uses a slightly different framing but must carry the
    // same seed and iterations.
    let debug = format!("{err:?}");
    let debug_hint = format!(
        "reproduce: noprop::Runner::new({:#018x}, {expected_iterations})",
        err.seed(),
    );
    assert!(
        debug.contains(&debug_hint),
        "Debug should contain reproduce hint {debug_hint:?}, got:\n{debug}"
    );

    // Using the hint verbatim should reproduce the same failure.
    let target = std::cell::Cell::new(0usize);
    let replay = noprop::Runner::new(err.seed(), expected_iterations).run(|_ctx| {
        let n = target.get();
        target.set(n + 1);
        if n >= 3 {
            panic!("boom at iteration {n}");
        }
        Ok(())
    });
    let replayed = replay.expect_err("hint iterations must reproduce the failure");
    assert_eq!(replayed.seed(), err.seed());
    assert_eq!(replayed.case_index(), err.case_index());
}

// === Stats ===

#[test]
fn stats_success_reports_accepted_iterations_and_zero_rejections() -> noprop::Result<()> {
    let mut runner = noprop::Runner::new(0xDEAD_BEEF, 10);
    runner.run(|ctx| {
        // Two sample_* per iteration => total_samples = 2 * iterations for a
        // clean run.
        let _a = noprop::sample_u32(ctx);
        let _b = noprop::sample_u32(ctx);
        Ok(())
    })?;
    let stats = runner.stats();
    assert_eq!(stats.accepted_iterations, 10);
    assert_eq!(stats.rejected_iterations, 0);
    assert_eq!(stats.total_samples, 20);
    Ok(())
}

#[test]
fn stats_counts_reject_case_unwinds() -> noprop::Result<()> {
    use std::cell::Cell;
    let counter = Cell::new(0usize);
    let mut runner = noprop::Runner::new(1, 3);
    runner.run(|ctx| {
        let n = counter.get();
        counter.set(n + 1);
        // First two invocations reject, then every subsequent one accepts.
        if n < 2 {
            ctx.reject_case();
        }
        Ok(())
    })?;
    let stats = runner.stats();
    assert_eq!(stats.accepted_iterations, 3);
    assert_eq!(stats.rejected_iterations, 2);
    Ok(())
}

#[test]
fn stats_counts_sample_with_rejection_exhaustion_as_rejected_iteration() {
    let mut runner = noprop::Runner::new(1, 1);
    let result = runner.run(|ctx| {
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
    assert_eq!(s.accepted_iterations, 0);
    assert!(
        s.rejected_iterations > 0,
        "expected some rejected iterations, got {}",
        s.rejected_iterations
    );
}

#[test]
fn stats_is_deterministic_per_seed() -> noprop::Result<()> {
    let mut a = noprop::Runner::new(42, 5);
    a.run(|ctx| {
        let _ = noprop::sample_u32(ctx);
        let _ = noprop::sample_bool(ctx);
        Ok(())
    })?;
    let mut b = noprop::Runner::new(42, 5);
    b.run(|ctx| {
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
    let err = noprop::Runner::new(7, 10)
        .run(|_ctx| {
            let n = counter.get();
            counter.set(n + 1);
            if n == 4 {
                panic!("boom at iteration {n}");
            }
            Ok(())
        })
        .expect_err("closure must fail at case 4");
    let stats = err.stats();
    // Four iterations passed before the panic on the fifth (index 4).
    assert_eq!(stats.accepted_iterations, 4);
    assert_eq!(stats.rejected_iterations, 0);
    assert_eq!(err.case_index(), stats.accepted_iterations);
}

// === Targeted PBT (Runner::run_targeted / TestCaseContext::maximize) ===

#[test]
fn run_targeted_succeeds_when_feedback_is_reported() {
    let mut runner = noprop::Runner::new(42, 64);
    runner
        .run_targeted(|ctx| {
            let x = noprop::sample_u32(ctx);
            ctx.maximize((x as f64) / u32::MAX as f64);
            Ok(())
        })
        .expect("targeted run with valid feedback must succeed");
    let stats = runner.stats();
    assert_eq!(stats.accepted_iterations, 64);
}

#[test]
fn run_targeted_missing_feedback_is_reported() {
    let err = noprop::Runner::new(1, 8)
        .run_targeted(|ctx| {
            let _ = noprop::sample_u32(ctx);
            // Deliberately no maximize call.
            Ok(())
        })
        .expect_err("accepted case without feedback must fail");
    let display = format!("{err}");
    assert!(
        display.contains("missing feedback"),
        "unexpected message: {display}"
    );
    assert!(
        display.contains("run_targeted"),
        "reproduce hint must name the targeted entry point: {display}"
    );
}

#[test]
fn run_targeted_invalid_feedback_is_reported() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let err = noprop::Runner::new(1, 8)
            .run_targeted(|ctx| {
                ctx.maximize(bad);
                Ok(())
            })
            .expect_err("NaN / infinity feedback must fail");
        let display = format!("{err}");
        assert!(
            display.contains("invalid feedback"),
            "unexpected message: {display}"
        );
    }
}

#[test]
fn maximize_is_noop_under_plain_run() {
    noprop::Runner::new(1, 8)
        .run(|ctx| {
            let x = noprop::sample_u32(ctx);
            ctx.maximize((x as f64) / u32::MAX as f64);
            Ok(())
        })
        .expect("maximize must be ignored by the plain runner");
}

#[test]
fn run_targeted_reports_property_failure() {
    let err = noprop::Runner::new(1, 32)
        .run_targeted(|_ctx| {
            panic!("deterministic failure");
        })
        .expect_err("panicking closure must fail the run");
    let display = format!("{err}");
    assert!(display.contains("deterministic failure"), "{display}");
    assert!(display.contains("run_targeted"), "{display}");
    assert_eq!(err.case_index(), 0);
}

#[test]
fn run_targeted_counts_rejections() {
    let mut runner = noprop::Runner::new(3, 16);
    runner
        .run_targeted(|ctx| {
            if noprop::sample_bool(ctx) {
                ctx.reject_case();
            }
            ctx.maximize(0.5);
            Ok(())
        })
        .expect("rejections must be retried like the plain runner");
    let stats = runner.stats();
    assert_eq!(stats.accepted_iterations, 16);
    assert!(stats.rejected_iterations > 0);
}

#[test]
fn run_targeted_with_span_based_generator_does_not_reject_everything() {
    let mut runner = noprop::Runner::new(11, 64);
    runner
        .run_targeted(|ctx| {
            let x = noprop::sample_usize_in(ctx, 0..10);
            ctx.maximize(x as f64 / 10.0);
            Ok(())
        })
        .expect("span-based generators must work under targeted search");
    let stats = runner.stats();
    assert_eq!(stats.accepted_iterations, 64);
    assert!(
        stats.rejected_iterations < 64,
        "exploratory candidates must not all be discarded: rejected={}",
        stats.rejected_iterations
    );
}

#[test]
fn run_targeted_with_choice_generator() {
    let mut runner = noprop::Runner::new(5, 64);
    runner
        .run_targeted(|ctx| {
            let idx = noprop::sample_choice(ctx, &[0usize, 1, 2, 3, 4]);
            ctx.maximize(idx as f64 / 4.0);
            Ok(())
        })
        .expect("choice-based generators must work under targeted search");
    let stats = runner.stats();
    assert_eq!(stats.accepted_iterations, 64);
    assert!(
        stats.rejected_iterations < 64,
        "choice candidates must not all be discarded: rejected={}",
        stats.rejected_iterations
    );
}

#[test]
fn run_targeted_stops_after_too_many_rejections() {
    let err = noprop::Runner::new(1, 8)
        .run_targeted(|ctx| {
            ctx.reject_case();
        })
        .expect_err("always-rejecting property must hit the rejection cap");
    let display = format!("{err}");
    assert!(display.contains("too many rejections"), "{display}");
    assert!(display.contains("run_targeted"), "{display}");
    let stats = err.stats();
    assert!(stats.rejected_iterations > 0);
}

#[test]
fn run_targeted_too_many_rejections_reproduces_with_the_hint_budget() {
    // The reproduce hint reuses the original iteration budget so the
    // rerun hits the same rejection cap and exits the same way.
    let seed = 0xBAD_CAFEu64;
    let run = || {
        noprop::Runner::new(seed, 16).run_targeted(|ctx| {
            ctx.reject_case();
        })
    };
    let err = run().expect_err("always-rejecting property must hit the rejection cap");
    let display = format!("{err}");
    assert!(display.contains("too many rejections"), "{display}");
    let replayed = run().expect_err("same seed and budget must reproduce the failure");
    assert_eq!(replayed.seed(), err.seed());
    assert_eq!(replayed.case_index(), err.case_index());
    assert_eq!(replayed.stats(), err.stats());
}

#[test]
fn run_targeted_reproduces_same_candidate_sequence() {
    use std::cell::Cell;
    let collect = |runner: &mut noprop::Runner| {
        let observed: Cell<Vec<usize>> = Cell::new(Vec::new());
        runner
            .run_targeted(|ctx| {
                let x = noprop::sample_usize_in(ctx, 0..1000);
                let mut v = observed.take();
                v.push(x);
                observed.set(v);
                ctx.maximize(x as f64 / 1000.0);
                Ok(())
            })
            .expect("targeted run");
        observed.into_inner()
    };
    let a = collect(&mut noprop::Runner::new(7, 64));
    let b = collect(&mut noprop::Runner::new(7, 64));
    assert_eq!(
        a, b,
        "candidate sequences must be reproducible from the seed"
    );
}

#[test]
fn run_targeted_reports_err_closure_failure() {
    let err = noprop::Runner::new(1, 32)
        .run_targeted(|ctx| {
            ctx.maximize(0.5);
            Err("application error".into())
        })
        .expect_err("returned Err must fail the run");
    let display = format!("{err}");
    assert!(display.contains("application error"), "{display}");
    assert!(display.contains("run_targeted"), "{display}");
}

#[test]
fn run_targeted_debug_output_reports_missing_feedback() {
    let err = noprop::Runner::new(1, 8)
        .run_targeted(|ctx| {
            let _ = noprop::sample_u32(ctx);
            Ok(())
        })
        .expect_err("missing feedback must fail");
    let debug = format!("{err:?}");
    assert!(debug.contains("missing_feedback: true"), "{debug}");
    assert!(debug.contains("run_targeted"), "{debug}");
}

#[test]
fn run_targeted_missing_feedback_reports_progress() {
    let err = noprop::Runner::new(1, 8)
        .run_targeted(|ctx| {
            let _ = noprop::sample_u32(ctx);
            Ok(())
        })
        .expect_err("missing feedback must fail");
    let stats = err.stats();
    assert_eq!(stats.accepted_iterations, 0);
    assert_eq!(stats.rejected_iterations, 0);
    assert_eq!(
        stats.total_samples, 1,
        "one sample before the missing report"
    );
    assert_eq!(err.case_index(), stats.accepted_iterations);
    assert!(
        !err.generated().is_empty(),
        "the failing case's generated trace must be recorded"
    );
}

#[test]
fn run_targeted_zero_iterations_does_not_invoke_closure() {
    use std::cell::Cell;
    let invoked = Cell::new(0usize);
    noprop::Runner::new(1, 0)
        .run_targeted(|ctx| {
            invoked.set(invoked.get() + 1);
            ctx.maximize(1.0);
            Ok(())
        })
        .expect("zero iterations must succeed");
    assert_eq!(invoked.get(), 0, "the closure must not be invoked");
}

#[test]
fn run_targeted_reproduce_hint_reproduces_the_same_failure() {
    // The failure condition (x >= 900, probability 0.1) must be found
    // within the budget for the hint test to be meaningful; 512
    // iterations leaves ample headroom even if the search constants
    // are retuned. The hint must reuse this exact budget.
    let seed = 0x5EED_1EAD_BEEF_C0DEu64;
    let iterations = 512;
    let run = || {
        noprop::Runner::new(seed, iterations).run_targeted(|ctx| {
            let x = noprop::sample_usize_in(ctx, 0..1000);
            ctx.maximize(x as f64 / 1000.0);
            if x >= 900 {
                panic!("boom at x = {x}");
            }
            Ok(())
        })
    };

    let err = run().expect_err("a large x must fail the run");
    let display = format!("{err}");
    let hint = format!(
        "reproduce with: noprop::Runner::new({:#018x}, {iterations}).run_targeted(|ctx| ...)",
        err.seed(),
    );
    assert!(
        display.contains(&hint),
        "Display should contain the targeted reproduce hint {hint:?}, got:\n{display}"
    );

    // Using the hint's budget verbatim reproduces the same failure.
    let replayed = run().expect_err("same seed and budget must reproduce the failure");
    assert_eq!(replayed.seed(), err.seed());
    assert_eq!(replayed.case_index(), err.case_index());
}

#[test]
fn run_targeted_failure_beats_invalid_feedback() {
    let err = noprop::Runner::new(1, 8)
        .run_targeted(|ctx| {
            ctx.maximize(f64::NAN);
            panic!("real failure");
        })
        .expect_err("property failure must win over invalid feedback");
    let display = format!("{err}");
    assert!(display.contains("real failure"), "{display}");
    assert!(!display.contains("invalid feedback"), "{display}");
}

#[test]
fn run_targeted_discards_score_of_rejected_cases() {
    let err = noprop::Runner::new(3, 8)
        .run_targeted(|ctx| {
            ctx.maximize(1.0);
            ctx.reject_case();
        })
        .expect_err("always-rejecting must hit the rejection cap, not missing feedback");
    let display = format!("{err}");
    assert!(display.contains("too many rejections"), "{display}");
    assert!(!display.contains("missing feedback"), "{display}");
}

fn shared_property(ctx: &mut noprop::TestCaseContext) -> Result<(), Box<dyn std::error::Error>> {
    let x = noprop::sample_usize_in(ctx, 0..1000);
    ctx.maximize(x as f64 / 1000.0);
    Ok(())
}

#[test]
fn same_property_runs_under_both_policies() {
    noprop::Runner::new(1, 16)
        .run(shared_property)
        .expect("uniform run must succeed");
    noprop::Runner::new(1, 16)
        .run_targeted(shared_property)
        .expect("targeted run must succeed");
}

#[test]
fn run_targeted_requires_feedback_after_rejected_case() {
    // A rejected case does not satisfy the accepted-case feedback
    // requirement: the next accepted case must still call maximize.
    // The seed makes the first case reject (asserted below) and a
    // later case reach the verdict without feedback, so the exit is
    // MissingFeedback — not TooManyRejections and not a silent Ok.
    let err = noprop::Runner::new(6, 8)
        .run_targeted(|ctx| {
            let x = noprop::sample_usize_in(ctx, 0..2);
            if x == 0 {
                ctx.reject_case();
            }
            Ok(())
        })
        .expect_err("accepted case without maximize must end in MissingFeedback");
    let display = format!("{err}");
    assert!(display.contains("missing feedback"), "{display}");
    assert!(
        err.stats().rejected_iterations >= 1,
        "the rejection path must be exercised before the accepted case: {:?}",
        err.stats()
    );
}

#[test]
fn run_targeted_counts_exploratory_draw_cap_excess_as_rejection() {
    // An exploratory case that draws past the recorded sequence
    // exceeds the generated-draw cap and is rejected; the run
    // continues and counts the rejection in the stats. The first
    // (recording) case draws once with this seed — asserted below —
    // so the recorded sequence stays one draw long; a later case
    // whose mutated draw is nonzero asks for 4100 draws (beyond
    // MAX_CHOICES_PER_CASE = 4096), of which only the first replays —
    // the rest must be generated, blowing past the cap. reject_case
    // is never called, so every counted rejection comes from the draw
    // cap.
    //
    // The first-case draw and the mutation behavior are fixed by the
    // seed and the search constants (MUTATION_DENOM, RANDOM_RESTART_
    // DENOM); re-selecting the seed is expected when tuning them.
    use std::cell::Cell;
    let mut runner = noprop::Runner::new(282, 8);
    let first_x: Cell<Option<u8>> = Cell::new(None);
    let result = runner.run_targeted(|ctx| {
        let x = noprop::sample_u8(ctx);
        if first_x.get().is_none() {
            first_x.set(Some(x));
        }
        ctx.maximize(x as f64 / u8::MAX as f64);
        if x != 0 {
            for _ in 0..4100 {
                noprop::sample_u8(ctx);
            }
        }
        Ok(())
    });
    assert!(
        result.is_ok(),
        "draw-cap rejections must not fail the run: {result:?}"
    );
    assert_eq!(
        first_x.get(),
        Some(0),
        "the recording case must draw zero so the recorded sequence stays short"
    );
    assert!(
        runner.stats().rejected_iterations >= 1,
        "draw-cap excess must count toward rejected_iterations"
    );
}

#[test]
fn run_targeted_steers_candidates_by_score() {
    // If the score were ignored (for example, a bug admitted every
    // case with a constant score), the observed candidate stream would
    // not depend on which end of the domain maximize rewards. It must:
    // rewarding large x keeps the search in the high end, rewarding
    // small x pins it to the low end, so the best observed candidate
    // in the second half of the run differs between the two.
    use std::cell::Cell;

    fn observe(seed: u64, score_of: fn(usize) -> f64) -> Vec<usize> {
        let observed: Cell<Vec<usize>> = Cell::new(Vec::new());
        noprop::Runner::new(seed, 256)
            .run_targeted(|ctx| {
                let x = noprop::sample_usize_in(ctx, 0..1000);
                let mut v = observed.take();
                v.push(x);
                observed.set(v);
                ctx.maximize(score_of(x));
                Ok(())
            })
            .expect("targeted run must succeed");
        observed.into_inner()
    }

    let second_half_max = |xs: &[usize]| xs[128..].iter().max().copied().unwrap_or(0);

    let reward_high = observe(6, |x| x as f64 / 1000.0);
    let reward_low = observe(6, |x| 1.0 - x as f64 / 1000.0);

    let high_max = second_half_max(&reward_high);
    let low_max = second_half_max(&reward_low);
    assert!(
        high_max > low_max,
        "the score must steer the search toward the rewarded end: \
         rewarding high reached {high_max}, rewarding low reached {low_max}"
    );
}

// === Corpus-guided PBT (Runner::run_corpus_guided / event / bucket / transition) ===

#[test]
fn run_corpus_guided_succeeds_with_features() {
    let mut runner = noprop::Runner::new(42, 64);
    runner
        .run_corpus_guided(|ctx| {
            let x = noprop::sample_u32(ctx);
            if x % 4 == 0 {
                ctx.event("multiple-of-four");
            }
            Ok(())
        })
        .expect("corpus-guided run with semantic feedback must succeed");
    let stats = runner.stats();
    assert_eq!(stats.accepted_iterations, 64);
}

#[test]
fn run_corpus_guided_without_features_succeeds() {
    // Corpus-guided mode does not require feedback: a property that
    // never reports a feature just yields no interesting cases.
    noprop::Runner::new(1, 8)
        .run_corpus_guided(|_ctx| Ok(()))
        .expect("a property without semantic feedback must not fail");
}

#[test]
fn run_corpus_guided_without_priority_succeeds() {
    // maximize is optional in corpus-guided mode; missing feedback is
    // not an error (unlike targeted mode).
    noprop::Runner::new(1, 8)
        .run_corpus_guided(|ctx| {
            let _ = noprop::sample_u32(ctx);
            ctx.event("e");
            Ok(())
        })
        .expect("a case without maximize must not fail corpus-guided mode");
}

#[test]
fn run_corpus_guided_invalid_priority_is_tolerated() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        noprop::Runner::new(1, 8)
            .run_corpus_guided(|ctx| {
                ctx.maximize(bad);
                Ok(())
            })
            .expect("NaN / infinity priority must not fail corpus-guided mode");
    }
}

#[test]
fn semantic_methods_are_noop_under_plain_run() {
    noprop::Runner::new(1, 8)
        .run(|ctx| {
            ctx.event("e");
            ctx.bucket("b", 1);
            ctx.transition("t", 0, 1);
            ctx.maximize(1.0);
            Ok(())
        })
        .expect("semantic methods must be ignored by the plain runner");
}

#[test]
fn run_corpus_guided_reports_property_failure() {
    let err = noprop::Runner::new(1, 32)
        .run_corpus_guided(|ctx| {
            ctx.event("before-failure");
            panic!("deterministic failure");
        })
        .expect_err("panicking closure must fail the run");
    let display = format!("{err}");
    assert!(display.contains("deterministic failure"), "{display}");
    assert!(display.contains("run_corpus_guided"), "{display}");
    assert!(display.contains("Semantic features:"), "{display}");
    assert!(display.contains("event(\"before-failure\")"), "{display}");
    assert_eq!(err.case_index(), 0);
}

#[test]
fn run_corpus_guided_candidate_index_is_one_based() {
    // The candidate index counts every attempt (accepted, rejected,
    // and the failing case itself) and is one-based, unlike the
    // zero-based accepted-iteration `case_index`.
    let attempts = std::cell::Cell::new(0usize);
    let err = noprop::Runner::new(1, 8)
        .run_corpus_guided(|ctx| {
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
fn run_corpus_guided_candidate_index_counts_rejected_attempts() {
    // A rejected attempt still advances the candidate index, so the
    // failing second attempt is candidate 2 while `case_index` stays 0
    // (no accepted iteration ran).
    let attempts = std::cell::Cell::new(0usize);
    let err = noprop::Runner::new(7, 8)
        .run_corpus_guided(|ctx| {
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
    assert_eq!(err.stats().rejected_iterations, 1);
}

#[test]
fn run_corpus_guided_reports_err_closure_failure_with_semantics() {
    let err = noprop::Runner::new(1, 8)
        .run_corpus_guided(|ctx| {
            ctx.event("before-error");
            Err("corpus error".into())
        })
        .expect_err("returned Err must fail the run");
    let display = format!("{err}");
    assert!(display.contains("corpus error"), "{display}");
    assert!(display.contains("run_corpus_guided"), "{display}");
    assert!(display.contains("Semantic features:"), "{display}");
    assert!(display.contains("event(\"before-error\")"), "{display}");
    assert_eq!(err.case_index(), 0);
    assert!(format!("{err:?}").contains("candidate_index: 1"), "{err:?}");
}

#[test]
fn run_corpus_guided_counts_rejections() {
    let mut runner = noprop::Runner::new(3, 16);
    runner
        .run_corpus_guided(|ctx| {
            let x = noprop::sample_u32(ctx);
            if x % 2 == 0 {
                ctx.reject_case();
            }
            ctx.event("accepted");
            Ok(())
        })
        .expect("rejections must be retried like the plain runner");
    let stats = runner.stats();
    assert_eq!(stats.accepted_iterations, 16);
    assert!(stats.rejected_iterations > 0);
}

#[test]
fn run_corpus_guided_stops_after_too_many_rejections() {
    let err = noprop::Runner::new(1, 8)
        .run_corpus_guided(|ctx| {
            ctx.reject_case();
        })
        .expect_err("always-rejecting property must hit the rejection cap");
    let display = format!("{err}");
    assert!(display.contains("too many rejections"), "{display}");
    assert!(display.contains("run_corpus_guided"), "{display}");
}

#[test]
fn run_corpus_guided_too_many_rejections_reports_last_rejected_semantics() {
    // The rejection-cap failure must carry the semantic features of
    // the last rejected case and its candidate index (every attempt
    // was rejected, so the index equals the rejected count). This
    // guards against the report silently dropping to "candidate_index:
    // 0" on this path.
    let err = noprop::Runner::new(1, 8)
        .run_corpus_guided(|ctx| {
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
        debug.contains(&format!(
            "candidate_index: {}",
            err.stats().rejected_iterations
        )),
        "candidate_index must equal the rejected count: {debug}"
    );
    let display = format!("{err}");
    assert!(display.contains("Semantic features:"), "{display}");
}

#[test]
fn run_corpus_guided_bounds_high_cardinality_features() {
    // A property that reports effectively unbounded bucket values must
    // not grow memory without bound: the per-case and global feature
    // caps keep the registry finite. The run succeeds regardless of how
    // many distinct values are reported.
    let mut runner = noprop::Runner::new(5, 64);
    runner
        .run_corpus_guided(|ctx| {
            let x = noprop::sample_u64(ctx);
            ctx.bucket("unbounded", x);
            ctx.bucket("also-unbounded", x.wrapping_add(1));
            ctx.bucket("yet-another", x.wrapping_mul(3));
            Ok(())
        })
        .expect("high-cardinality features must be capped, not fatal");
    let stats = runner.stats();
    assert_eq!(stats.accepted_iterations, 64);
}

#[test]
fn run_corpus_guided_rejected_cases_register_features() {
    // A rejected case that reported a novel feature before rejecting
    // must be admitted into the rejected queue (low-energy
    // scaffolding), so its features count toward the global registry.
    // This is observable through the run's behaviour: the same feature
    // reported later by an accepted case is no longer novel.
    let mut runner = noprop::Runner::new(11, 32);
    runner
        .run_corpus_guided(|ctx| {
            let x = noprop::sample_u32(ctx);
            ctx.event("shared-feature");
            if x % 2 == 0 {
                ctx.reject_case();
            }
            Ok(())
        })
        .expect("rejected cases with novel features must be tolerated");
    let stats = runner.stats();
    assert_eq!(stats.accepted_iterations, 32);
    assert!(stats.rejected_iterations > 0);
}

#[test]
fn run_corpus_guided_is_reproducible_from_seed() {
    let seed = 0xABCD_EF01_2345_6789;

    fn run(seed: u64) -> Vec<u32> {
        let observed = std::cell::Cell::new(Vec::new());
        noprop::Runner::new(seed, 64)
            .run_corpus_guided(|ctx| {
                let x = noprop::sample_u32(ctx);
                let mut v = observed.take();
                v.push(x);
                observed.set(v);
                if x % 2 == 0 {
                    ctx.event("even");
                }
                Ok(())
            })
            .expect("corpus-guided run must succeed");
        observed.into_inner()
    }

    assert_eq!(run(seed), run(seed));
}

#[test]
fn run_corpus_guided_with_rejections_is_reproducible_from_seed() {
    // The plain reproducibility test never rejects, so the
    // rejected-queue pick path (and the PRNG rolls it consumes) stays
    // unexercised. This property rejects a fraction of candidates so
    // the full candidate stream — including the rejected-queue branch
    // of `next_context` — must reproduce from the seed.
    use std::cell::Cell;

    fn run(seed: u64) -> Vec<u32> {
        let observed: Cell<Vec<u32>> = Cell::new(Vec::new());
        noprop::Runner::new(seed, 64)
            .run_corpus_guided(|ctx| {
                let x = noprop::sample_u32(ctx);
                let mut v = observed.take();
                v.push(x);
                observed.set(v);
                if x % 2 == 0 {
                    ctx.event("even");
                }
                if x % 4 == 0 {
                    ctx.reject_case();
                }
                Ok(())
            })
            .expect("corpus-guided run must succeed");
        observed.into_inner()
    }

    let seed = 0xC0FFEE;
    let a = run(seed);
    let b = run(seed);
    assert_eq!(a, b, "rejected candidates must reproduce from the seed");
    assert!(
        a.iter().any(|x| x % 4 == 0),
        "the rejected path must actually run: {a:?}"
    );
}

#[test]
fn run_corpus_guided_stats_count_rejected_attempt_samples() {
    // `total_samples` must include samples produced by rejected
    // attempts (they consumed generator budget), and
    // `rejected_iterations` must count every `reject_case`.
    use std::cell::Cell;

    let attempts = Cell::new(0usize);
    let mut runner = noprop::Runner::new(1, 4);
    runner
        .run_corpus_guided(|ctx| {
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
    assert_eq!(stats.accepted_iterations, 4);
    assert_eq!(stats.rejected_iterations, 1);
    assert_eq!(
        stats.total_samples, 5,
        "1 rejected + 4 accepted attempts, one sample each"
    );
}

#[test]
fn run_corpus_guided_steers_candidates_by_novel_features() {
    // The search must reproduce the interesting input region: a
    // property that reports a feature only when x is large keeps
    // exploring inputs that reach that feature, so the best observed
    // candidate in the second half of the run is high.
    use std::cell::Cell;

    let observed: Cell<Vec<usize>> = Cell::new(Vec::new());
    noprop::Runner::new(7, 256)
        .run_corpus_guided(|ctx| {
            let x = noprop::sample_usize_in(ctx, 0..1000);
            let mut v = observed.take();
            v.push(x);
            observed.set(v);
            if x > 900 {
                ctx.event("high");
            }
            Ok(())
        })
        .expect("corpus-guided run must succeed");
    let xs = observed.into_inner();
    let second_half_max = xs[128..].iter().max().copied().unwrap_or(0);
    assert!(
        second_half_max > 900,
        "the corpus must steer the search toward the feature region: \
         second-half max reached {second_half_max}"
    );
}

#[test]
fn run_corpus_guided_steers_candidates_by_priority() {
    // With scalar priority enabled, a case that reports no novel
    // feature can still be admitted by outscoring its feature group, so
    // rewarding large x must keep the search in the high end just like
    // targeted mode.
    use std::cell::Cell;

    fn observe(seed: u64, score_of: fn(usize) -> f64) -> Vec<usize> {
        let observed: Cell<Vec<usize>> = Cell::new(Vec::new());
        noprop::Runner::new(seed, 256)
            .run_corpus_guided(|ctx| {
                let x = noprop::sample_usize_in(ctx, 0..1000);
                let mut v = observed.take();
                v.push(x);
                observed.set(v);
                ctx.event("covered");
                ctx.maximize(score_of(x));
                Ok(())
            })
            .expect("corpus-guided run must succeed");
        observed.into_inner()
    }

    let second_half_max = |xs: &[usize]| xs[128..].iter().max().copied().unwrap_or(0);

    let reward_high = observe(6, |x| x as f64 / 1000.0);
    let reward_low = observe(6, |x| 1.0 - x as f64 / 1000.0);

    let high_max = second_half_max(&reward_high);
    let low_max = second_half_max(&reward_low);
    assert!(
        high_max > low_max,
        "the priority must steer the search toward the rewarded end: \
         rewarding high reached {high_max}, rewarding low reached {low_max}"
    );
}

#[test]
fn run_corpus_guided_steers_stateful_transitions() {
    // A stateful-style target: the property advances an abstract state
    // machine and reports each transition. The corpus must keep
    // exploring cases that reach the deep transitions, so the deepest
    // chain observed under corpus-guided search is deeper than under
    // uniform sampling with the same seed cohort.
    use std::cell::Cell;

    fn observe(seed: u64, corpus_guided: bool) -> Vec<usize> {
        let observed: Cell<Vec<usize>> = Cell::new(Vec::new());
        let mut runner = noprop::Runner::new(seed, 256);
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
                .run_corpus_guided(property)
                .expect("corpus-guided run must succeed");
        } else {
            runner.run(property).expect("uniform run must succeed");
        }
        observed.into_inner()
    }

    let second_half_max = |xs: &[usize]| xs[128..].iter().max().copied().unwrap_or(0);

    let uniform_depths = observe(9, false);
    let corpus_depths = observe(9, true);
    let uniform_max = second_half_max(&uniform_depths);
    let corpus_max = second_half_max(&corpus_depths);
    assert!(
        corpus_max > uniform_max,
        "the corpus must explore deeper transition chains than uniform sampling: \
         uniform reached {uniform_max}, corpus reached {corpus_max}"
    );
}
