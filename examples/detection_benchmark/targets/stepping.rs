//! Sparse-precondition target: the mutant fails only when five
//! consecutive draws are all zero. Uniform sampling reaches the
//! witness with probability 1e-5 per case; the biased variant draws
//! zero with probability 0.7. The progress toward the precondition
//! (the number of leading zero draws) is observable as a bucket, so
//! the search policies can be seen steering along the progress axis.

use super::{Observe, Task, Workload};
use noprop::TestCaseContext;

fn run(
    sut_mutant: bool,
    biased: bool,
    ctx: &mut TestCaseContext,
    _obs: &Observe,
) -> Result<(), String> {
    let mut progress = 0u64;
    for _ in 0..5 {
        let x = if biased {
            // 70% exactly zero, otherwise 1..10 (never zero).
            if noprop::sample_usize_in(ctx, 0..10) < 7 {
                0
            } else {
                noprop::sample_usize_in(ctx, 1..10)
            }
        } else {
            noprop::sample_usize_in(ctx, 0..10)
        };
        if x != 0 {
            break;
        }
        progress += 1;
    }
    // Feedback: the mutant fails at full progress, so the bucket
    // observes the progress in finite steps. This call draws no random
    // bytes, so the generator stream is unchanged.
    ctx.bucket("progress", progress);
    if sut_mutant && progress == 5 {
        return Err("all five consecutive draws were zero".to_string());
    }
    Ok(())
}

pub(crate) const WORKLOAD: Workload = Workload {
    name: "stepping",
    description: "mutant fails only after five consecutive zero draws; progress is observable",
    tasks: &[Task {
        mutant: "fails_on_five_zeros",
        base: run_base,
        uniform: run_uniform,
        biased: run_biased,
        // The generator draws only bounded ranges, which the generic
        // boundary mix does not wrap, so the bb property is the
        // uniform one.
        bb: run_uniform,
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
