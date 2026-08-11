//! Guard target: reports effectively unbounded bucket values on every
//! case to verify that the feedback-guided machinery stays bounded (the
//! global feature registry saturates at 1024 and the corpus at 64)
//! and deterministic across all variants. No mutant: detection is
//! never expected, and every variant must complete the run.

use super::{Observe, Task, Workload};

fn run_case(
    _sut_mutant: bool,
    draw: fn(&mut noprop::TestCaseContext) -> u64,
    ctx: &mut noprop::TestCaseContext,
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

fn run_case_base(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_case(false, noprop::sample_u64, ctx, obs)
}
fn run_case_uniform(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_case(true, noprop::sample_u64, ctx, obs)
}
// guard has no mutant, so there is no failure region to bias toward:
// the biased and uniform variants deliberately share the same draw
// distribution. Keeping the biased slot lets guard run under every
// variant of the benchmark harness (registry saturation, determinism)
// without carving out a special case.
fn run_case_biased(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_case(true, noprop::sample_u64, ctx, obs)
}
fn run_case_bb(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_case(true, crate::bb::u64, ctx, obs)
}
