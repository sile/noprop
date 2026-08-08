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
    x: u32,
    y: u32,
    ctx: &mut TestCaseContext,
    _obs: &Observe,
) -> Result<(), String> {
    // Feedback: the mutant fails for the exact pair (1, 2), so the
    // events observe the two conditions separately. These calls draw
    // no random bytes, so the generator stream is unchanged.
    ctx.event(if x == 1 { "x_witness" } else { "x_other" });
    ctx.event(if y == 2 { "y_witness" } else { "y_other" });
    process(x, y, sut_mutant).map_err(|_| format!("process failed for ({x}, {y})"))
}

fn run_base(ctx: &mut TestCaseContext, obs: &Observe) -> Result<(), String> {
    let x = draw(None, ctx);
    let y = draw(None, ctx);
    run(false, x, y, ctx, obs)
}
fn run_uniform(ctx: &mut TestCaseContext, obs: &Observe) -> Result<(), String> {
    let x = draw(None, ctx);
    let y = draw(None, ctx);
    run(true, x, y, ctx, obs)
}
fn run_biased(ctx: &mut TestCaseContext, obs: &Observe) -> Result<(), String> {
    let x = draw(Some(1), ctx);
    let y = draw(Some(2), ctx);
    run(true, x, y, ctx, obs)
}
fn run_bb(ctx: &mut TestCaseContext, obs: &Observe) -> Result<(), String> {
    let x = crate::bb::u32(ctx);
    let y = crate::bb::u32(ctx);
    run(true, x, y, ctx, obs)
}

pub(crate) const WORKLOAD: Workload = Workload {
    name: "combination",
    description: "mutant fails only for the exact draw pair (1, 2)",
    tasks: &[Task {
        mutant: "fails_on_specific_pair",
        base: run_base,
        uniform: run_uniform,
        biased: run_biased,
        bb: run_bb,
    }],
};
