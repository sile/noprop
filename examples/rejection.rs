//! Rejection: the two ways to discard input, and when to use which.
//!
//! - `sample_with_rejection(ctx, max_attempts, attempt)`: retry a
//!   *single constrained draw* (an even number, a non-empty string,
//!   ...) up to `max_attempts` times. The attempt boundary stays in
//!   the choice sequence.
//! - `TestCaseContext::reject_case()`: discard the *whole case* when
//!   its preconditions are violated after sampling (a sparse
//!   semantic constraint). Rejected cases are retried and do not
//!   count toward the case budget.
//!
//! Run with: `cargo run --example rejection`

use std::cell::Cell;

fn main() -> noprop::TestResult {
    // === Constrained draw: sample_with_rejection ===
    //
    // Every value must be a multiple of 4. Keeping the rejection
    // local to the draw avoids regenerating the whole case.
    let mut runner = noprop::Runner::new(0xDEAD_BEEF);
    runner.run(64, |ctx| {
        let v = noprop::sample_with_rejection(ctx, 8, |ctx| {
            let x = noprop::sample_u32(ctx);
            x.is_multiple_of(4).then_some(x)
        });
        assert!(v.is_multiple_of(4));
        Ok(())
    })?;
    println!(
        "constrained draw: passed (rejected {} cases)",
        runner.stats().rejected_cases
    );

    // === Whole-case precondition: reject_case ===
    //
    // The property only makes sense for pairs `(a, b)` with `a < b`;
    // a draw violating the precondition is rejected after sampling.
    // The rejection budget is bounded, so an always-invalid
    // generator still terminates with a TooManyRejections error.
    let attempts = Cell::new(0usize);
    let mut runner = noprop::Runner::new(0xFEED);
    runner.run(32, |ctx| {
        attempts.set(attempts.get() + 1);
        let a = noprop::sample_usize_in(ctx, 0..100);
        let b = noprop::sample_usize_in(ctx, 0..100);
        if a >= b {
            ctx.reject_case();
        }
        assert!(a < b, "precondition must hold: {a} < {b}");
        Ok(())
    })?;
    println!(
        "whole-case rejection: passed ({} attempts for 32 accepted cases)",
        attempts.get()
    );
    Ok(())
}
