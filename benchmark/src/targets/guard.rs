//! Guard target: reports effectively unbounded bucket values on every
//! case to verify that the feedback-guided machinery stays bounded (the
//! global feature registry saturates at 1024 and the corpus at 64)
//! and deterministic across all variants. No mutant: detection is
//! never expected, and every variant must complete the run.

use super::{Observe, Task, Workload};
use noprop::TestCaseContext;

fn run_case(
    _sut_mutant: bool,
    draw: fn(&mut TestCaseContext) -> u64,
    ctx: &mut TestCaseContext,
    _obs: &Observe,
) -> Result<(), String> {
    // Report fresh near-uniform values on every case: a property that
    // misuses `bucket` with an unbounded value must not grow memory
    // without bound or abort the run. The per-case cap truncates the
    // reports and the global registry saturates.
    for _ in 0..64 {
        let v = draw(ctx);
        ctx.bucket("noise", v);
    }
    Ok(())
}

pub(crate) const WORKLOAD: Workload = Workload {
    name: "guard",
    description: "reports unbounded bucket values; checks corpus bounds and determinism",
    tasks: &[Task {
        mutant: "reports_unbounded_buckets",
        base: run_case_base,
        uniform: run_case_uniform,
        biased: run_case_biased,
        bb: run_case_bb,
    }],
};

fn run_case_base(ctx: &mut TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_case(false, noprop::sample_u64, ctx, obs)
}
fn run_case_uniform(ctx: &mut TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_case(true, noprop::sample_u64, ctx, obs)
}
fn run_case_biased(ctx: &mut TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_case(true, noprop::sample_u64, ctx, obs)
}
fn run_case_bb(ctx: &mut TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_case(true, crate::bb::u64, ctx, obs)
}
