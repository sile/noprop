//! Stateful transition target: a synthetic command-sequence workload.
//! Each case applies `advance` / `reset` commands to an abstract
//! state; the mutant fails once the state reaches 7. Every case runs
//! the same code path — only the transition combination differs, which
//! is exactly what feedback-guided search observes.

use super::{Observe, Task, Workload};

fn run(
    sut_mutant: bool,
    biased: bool,
    ctx: &mut noprop::TestCaseContext,
    _obs: &Observe,
) -> Result<(), String> {
    let mut state = 0u64;
    let steps = noprop::sample_usize_in(ctx, 0..32);
    for _ in 0..steps {
        let advance = if biased {
            // 95% advance, so seven consecutive advances are common.
            noprop::sample_usize_in(ctx, 0..20) < 19
        } else {
            noprop::sample_bool(ctx)
        };
        let next = if advance { state + 1 } else { 0 };
        // Feedback: the transitions observe the path taken (the same
        // code path for every case); this call draws no random bytes,
        // so the generator stream is unchanged.
        ctx.transition("state", state, next);
        state = next;
        if sut_mutant && state >= 7 {
            return Err(format!("state reached {state}"));
        }
    }
    Ok(())
}

pub(crate) const WORKLOAD: Workload = Workload {
    name: "stateful",
    description: "mutant fails when the abstract state reaches 7; transitions are observable",
    tasks: &[Task {
        mutant: "fails_on_state_seven",
        base: run_base,
        uniform: run_uniform,
        biased: run_biased,
        // The generator draws only bounded ranges and booleans, which
        // the generic boundary mix does not wrap, so the bb property
        // is the uniform one.
        bb: run_uniform,
    }],
};

fn run_base(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(false, false, ctx, obs)
}
fn run_uniform(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(true, false, ctx, obs)
}
fn run_biased(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(true, true, ctx, obs)
}
