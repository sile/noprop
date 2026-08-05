//! High-frequency target: the mutant fails for roughly half of the
//! uniform input space, so every variant detects it; the biased
//! variant steers toward the failing half and detects it earlier.

use super::{Observe, Task, Workload};
use noprop::TestCaseContext;

/// SUT: `process(x)` succeeds for even `x`.
fn process(x: u32, mutant: bool) -> Result<(), ()> {
    if mutant && !x.is_multiple_of(2) {
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
        // 90% odd, 10% even: the failing half is over-represented.
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
    };
    process(x, sut_mutant).map_err(|_| format!("process failed for x={x}"))
}

pub(crate) const WORKLOAD: Workload = Workload {
    name: "high-frequency",
    description: "mutant fails for roughly half of the uniform input space",
    tasks: &[Task {
        mutant: "fails_on_odd",
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
