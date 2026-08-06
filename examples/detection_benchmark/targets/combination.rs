//! Combination target: the mutant fails only for the exact pair
//! `(x, y) == (1, 2)`, which uniform sampling never reaches; the
//! biased variant steers both draws toward the witness values.

use super::{Observe, Task, Workload};
use noprop::TestCaseContext;

/// SUT: `process(x, y)` succeeds unless the pair is `(1, 2)` under the
/// mutant.
fn process(x: u32, y: u32, mutant: bool) -> Result<(), ()> {
    if mutant && x == 1 && y == 2 {
        return Err(());
    }
    Ok(())
}

/// Draw one input value: 50% the witness value when biased, otherwise
/// uniform.
fn draw(witness: Option<u32>, ctx: &mut TestCaseContext) -> u32 {
    match witness {
        Some(w) if noprop::sample_bool(ctx) => w,
        _ => noprop::sample_u32(ctx),
    }
}

fn run(
    sut_mutant: bool,
    biased: bool,
    ctx: &mut TestCaseContext,
    _obs: &Observe,
) -> Result<(), String> {
    let x = draw(if biased { Some(1) } else { None }, ctx);
    let y = draw(if biased { Some(2) } else { None }, ctx);
    // Feedback: the mutant fails for the exact pair (1, 2), so the
    // priority rewards each draw landing on its witness value and the
    // events observe the two conditions separately. These calls draw
    // no random bytes, so the generator stream is unchanged.
    ctx.event(if x == 1 { "x_witness" } else { "x_other" });
    ctx.event(if y == 2 { "y_witness" } else { "y_other" });
    ctx.maximize((if x == 1 { 0.5 } else { 0.0 }) + (if y == 2 { 0.5 } else { 0.0 }));
    process(x, y, sut_mutant).map_err(|_| format!("process failed for ({x}, {y})"))
}

pub(crate) const WORKLOAD: Workload = Workload {
    name: "combination",
    description: "mutant fails only for the exact draw pair (1, 2)",
    tasks: &[Task {
        mutant: "fails_on_specific_pair",
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
