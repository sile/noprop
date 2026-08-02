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
        iterations: 16,
    }
    .run(|rng| {
        let x = noprop::sample_u32(rng);
        assert_eq!(x, x);
        Ok(())
    })
}

#[test]
fn run_returns_err_on_failed_assertion() {
    // Property "every u32 is zero" fails almost immediately.
    let result = noprop::Runner {
        seed: 0x1234,
        iterations: 64,
    }
    .run(|rng| {
        let x = noprop::sample_u32(rng);
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
        noprop::Runner {
            seed,
            iterations: 32,
        }
        .run(|rng| {
            let x = noprop::sample_u32(rng);
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
    noprop::Runner {
        seed: 0,
        iterations: 0,
    }
    .run(|_rng| {
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
    let result = noprop::Runner {
        seed,
        iterations: 1,
    }
    .run(|_rng| {
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
    let _ = noprop::Runner {
        seed: 0,
        iterations: 100,
    }
    .run(|_rng| {
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
    let result = noprop::Runner {
        seed: 42,
        iterations: 1,
    }
    .run(|rng| {
        let x = noprop::sample_u32(rng);
        let b = noprop::sample_bool(rng);
        let c = noprop::sample_ascii_char(rng);
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
    let result = noprop::Runner {
        seed: 1,
        iterations: 1,
    }
    .run(|rng| {
        for _ in 0..100 {
            let _ = noprop::sample_u8(rng);
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
    let result = noprop::Runner {
        seed: 1,
        iterations: 1,
    }
    .run(|rng| {
        for _ in 0..16 {
            let _ = noprop::sample_u8(rng);
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
    let result = noprop::Runner {
        seed: 1,
        iterations: 1,
    }
    .run(|rng| {
        for _ in 0..3 {
            let _ = noprop::sample_u8(rng);
        }
        for _ in 0..100 {
            let _ = noprop::sample_u16(rng);
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
    let result = noprop::Runner {
        seed: 7,
        iterations: 5,
    }
    .run(|rng| {
        let c = case.get();
        if c == 0 {
            let _ = noprop::sample_u64(rng);
        } else {
            let _ = noprop::sample_u16(rng);
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
    let result = noprop::Runner {
        seed: 42,
        iterations: 1,
    }
    .run(|rng| {
        let _ = noprop::sample_u32(rng);
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
    let result = noprop::Runner {
        seed: 5,
        iterations: 1,
    }
    .run(|rng| {
        let _key: [u8; 16] = noprop::sample_bytes(rng);
        panic!("stop");
    });
    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 1);
    assert_eq!(generated[0].type_name(), "[u8; 16]");
}

#[test]
fn sample_bytes_vec_records_the_vec_as_one_trace_entry() {
    let result = noprop::Runner {
        seed: 5,
        iterations: 1,
    }
    .run(|rng| {
        let _buf = noprop::sample_bytes_vec(rng, 42);
        panic!("stop");
    });
    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 1);
    assert_eq!(generated[0].type_name(), "alloc::vec::Vec<u8>");
}

#[test]
fn error_display_output_includes_generated_values() {
    let result = noprop::Runner {
        seed: 42,
        iterations: 1,
    }
    .run(|rng| {
        let _ = noprop::sample_u8(rng);
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
    let result = noprop::Runner {
        seed: 5,
        iterations: 1,
    }
    .run(|rng| {
        let _v = noprop::sample_usize_in(rng, 0..7);
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
    let result = noprop::Runner {
        seed: 5,
        iterations: 1,
    }
    .run(|rng| {
        let _b = noprop::sample_ratio(rng, 1, 3);
        panic!("stop");
    });
    let err = result.expect_err("expected Err");
    let generated = err.generated();
    assert_eq!(generated.len(), 1, "generated: {generated:?}");
    assert_eq!(generated[0].type_name(), "bool");
}

#[test]
fn sample_weighted_index_records_only_the_chosen_index() {
    let result = noprop::Runner {
        seed: 5,
        iterations: 1,
    }
    .run(|rng| {
        let _idx = noprop::sample_weighted_index(rng, &[1, 2, 3, 4]);
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
        noprop::Runner {
            seed,
            iterations: 64,
        }
        .run(|rng| {
            let idx = noprop::sample_weighted_index(rng, &[1, 1, 1, 1]);
            let n = noprop::sample_usize_in(rng, 0..=100);
            let flip = noprop::sample_ratio(rng, 1, 4);
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
    noprop::Runner {
        seed: 1,
        iterations: 8,
    }
    .run(|rng| {
        let v = noprop::sample_with_rejection(rng, 4, |rng| {
            let x = noprop::sample_u32(rng);
            x.is_multiple_of(2).then_some(x)
        });
        assert!(v.is_multiple_of(2));
        Ok(())
    })
}

#[test]
fn reject_case_retries_iteration_without_counting_it() -> noprop::Result<()> {
    // Reject on the first N iterations then succeed forever. All N
    // rejections must not consume the iterations budget.
    let attempts = std::cell::Cell::new(0usize);
    let accepted = std::cell::Cell::new(0usize);
    let target_accepts = 4;
    noprop::Runner {
        seed: 42,
        iterations: target_accepts,
    }
    .run(|rng| {
        let n = attempts.get();
        attempts.set(n + 1);
        if n < 3 {
            rng.reject_case();
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
    let result = noprop::Runner {
        seed: 7,
        iterations: 8,
    }
    .run(|rng| {
        rng.reject_case();
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
        noprop::Runner {
            seed,
            iterations: 4,
        }
        .run(|rng| {
            rng.reject_case();
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
    noprop::Runner {
        seed: 1,
        iterations: 2,
    }
    .run(|rng| {
        let n = attempts.get();
        attempts.set(n + 1);
        if n == 0 {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                rng.reject_case();
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
    noprop::Runner {
        seed: 1,
        iterations: 1,
    }
    .run(|rng| {
        let n = attempts.get();
        attempts.set(n + 1);
        if n == 0 {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                rng.reject_case();
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
    noprop::Runner {
        seed: 1,
        iterations: 2,
    }
    .run(|rng| {
        let n = outer_attempts.get();
        outer_attempts.set(n + 1);
        if n < 2 {
            // First two invocations reject via sample_with_rejection exhaustion.
            let _: u32 = noprop::sample_with_rejection(rng, 4, |_rng| None);
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
    let mut rng = noprop::Rng::new(0);
    rng.reject_case();
}

// === sample_string / sample_ascii_string / sample_ascii_printable_string trace format ===

#[test]
fn sample_string_records_one_entry_per_call() {
    let result = noprop::Runner {
        seed: 11,
        iterations: 1,
    }
    .run(|rng| {
        let a = noprop::sample_string(rng, 4);
        let b = noprop::sample_ascii_string(rng, 4);
        let c = noprop::sample_ascii_printable_string(rng, 4);
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
    let result = noprop::Runner {
        seed: 5,
        iterations: 1,
    }
    .run(|rng| {
        let a = noprop::sample_f32_finite(rng);
        let b = noprop::sample_f64_finite(rng);
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
        noprop::Runner {
            seed,
            iterations: 128,
        }
        .run(|_rng| {
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
    let expected_iterations = err.case_index() + 1;
    let hint = format!(
        "reproduce with: noprop::Runner {{ seed: {:#018x}, iterations: {expected_iterations} }}",
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
        "reproduce: noprop::Runner {{ seed: {:#018x}, iterations: {expected_iterations} }}",
        err.seed(),
    );
    assert!(
        debug.contains(&debug_hint),
        "Debug should contain reproduce hint {debug_hint:?}, got:\n{debug}"
    );

    // Using the hint verbatim should reproduce the same failure.
    let target = std::cell::Cell::new(0usize);
    let replay = noprop::Runner {
        seed: err.seed(),
        iterations: expected_iterations,
    }
    .run(|_rng| {
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
