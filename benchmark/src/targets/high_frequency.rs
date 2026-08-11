//! High-frequency target: the mutant fails for roughly half of the
//! uniform input space, so every variant detects it; the biased
//! variant steers toward the failing half and detects it earlier.

use super::{Observe, Task, Workload};

/// SUT: `process(x)` succeeds for even `x`.
fn process(x: u32, mutant: bool) -> Result<(), ()> {
    if mutant && !x.is_multiple_of(2) {
        return Err(());
    }
    Ok(())
}

fn run(
    sut_mutant: bool,
    x: u32,
    ctx: &mut noprop::TestCaseContext,
    _obs: &Observe,
) -> Result<(), String> {
    // Feedback: the mutant fails for odd x, so the event observes
    // which parity was reached. This call draws no random bytes, so
    // the generator stream is unchanged.
    ctx.event(if x.is_multiple_of(2) { "even" } else { "odd" });
    process(x, sut_mutant).map_err(|_| format!("process failed for x={x}"))
}

/// Draw one input value: 90% odd, 10% even when biased (the failing
/// half is over-represented), otherwise uniform.
fn draw(biased: bool, ctx: &mut noprop::TestCaseContext) -> u32 {
    if biased {
        let odd = noprop::sample_usize_in(ctx, 0..10) < 9;
        let mut v = noprop::sample_u32(ctx);
        if odd {
            v |= 1;
        } else {
            v &= !1;
        }
        v
    } else {
        noprop::sample_u32(ctx)
    }
}

fn run_base(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(false, draw(false, ctx), ctx, obs)
}
fn run_uniform(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(true, draw(false, ctx), ctx, obs)
}
fn run_biased(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(true, draw(true, ctx), ctx, obs)
}
fn run_bb(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(true, crate::bb::u32(ctx), ctx, obs)
}

pub(crate) const WORKLOAD: Workload = Workload {
    name: "high-frequency",
    description: "mutant fails for roughly half of the uniform input space",
    tasks: &[Task {
        mutant: "fails_on_odd",
        base: run_base,
        uniform: run_uniform,
        biased: run_biased,
        bb: run_bb,
    }],
};
