//! Boundary target: the mutant fails only for `x == 0`, which uniform
//! sampling essentially never draws; the biased variant makes zero
//! reachable and detects it.

use super::{Observe, Task, Workload};
use noprop::TestCaseContext;

/// SUT: `process(x)` succeeds unless `x == 0` under the mutant.
fn process(x: u32, mutant: bool) -> Result<(), ()> {
    if mutant && x == 0 {
        return Err(());
    }
    Ok(())
}

fn run(
    sut_mutant: bool,
    biased: bool,
    ctx: &mut TestCaseContext,
    _obs: &Observe,
) -> Result<(), String> {
    let x = if biased {
        // 10% exactly zero, 90% uniform.
        if noprop::sample_usize_in(ctx, 0..10) == 0 {
            0
        } else {
            noprop::sample_u32(ctx)
        }
    } else {
        noprop::sample_u32(ctx)
    };
    // Feedback: the mutant fails for x == 0, so the priority rewards
    // values close to zero (measured by leading zeros, 0..=32) and the
    // bucket observes that distance in finite steps. These calls draw
    // no random bytes, so the generator stream is unchanged.
    let zeros = x.leading_zeros();
    ctx.bucket("leading_zeros", zeros as u64);
    ctx.maximize(zeros as f64 / 32.0);
    process(x, sut_mutant).map_err(|_| format!("process failed for x={x}"))
}

pub(crate) const WORKLOAD: Workload = Workload {
    name: "boundary",
    description: "mutant fails only for the boundary value zero",
    tasks: &[Task {
        mutant: "fails_on_zero",
        base: run_base,
        uniform: run_uniform,
        biased: run_biased,
    }],
};

fn run_base(ctx: &mut TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(false, false, ctx, obs)
}
fn run_uniform(ctx: &mut TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(true, false, ctx, obs)
}
fn run_biased(ctx: &mut TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(true, true, ctx, obs)
}
