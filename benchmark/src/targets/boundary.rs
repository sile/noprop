//! Boundary targets: the mutants fail only for specific values that
//! uniform sampling essentially never draws. Three flavors isolate
//! what an automatic type-level boundary bias can reach:
//!
//! - `fails_on_zero`: the witness `0` is a type-level boundary, so
//!   both the explicitly biased generator and the generic boundary mix
//!   can reach it.
//! - `fails_on_domain_value`: the witness `1500` is a domain constant
//!   outside the type-level boundary set, so only the explicitly
//!   biased generator (which lists the domain value) reaches it.
//! - `fails_on_range_end`: the witness `1023` is the end of a bounded
//!   range draw, which the generic boundary mix does not wrap, so only
//!   the explicitly biased generator reaches it.

use super::{Observe, Task, Workload};

/// SUT: `process(x)` succeeds unless `x == 0` under the mutant.
fn process(x: u32, mutant: bool) -> Result<(), ()> {
    if mutant && x == 0 {
        return Err(());
    }
    Ok(())
}

/// SUT: `process_domain(x)` succeeds unless `x == 1500` under the
/// mutant.
fn process_domain(x: u32, mutant: bool) -> Result<(), ()> {
    if mutant && x == 1500 {
        return Err(());
    }
    Ok(())
}

/// SUT: `process_range(v)` succeeds unless `v == 1023` under the
/// mutant.
fn process_range(v: usize, mutant: bool) -> Result<(), ()> {
    if mutant && v == 1023 {
        return Err(());
    }
    Ok(())
}

fn run(
    sut_mutant: bool,
    biased: bool,
    ctx: &mut noprop::TestCaseContext,
    _obs: &Observe,
) -> Result<(), String> {
    let x = if biased {
        // 10% exactly zero, 90% uniform.
        noprop::sample_with_boundaries(ctx, &[0], noprop::Ratio::one_nth(10), noprop::sample_u32)
    } else {
        noprop::sample_u32(ctx)
    };
    process(x, sut_mutant).map_err(|_| format!("process failed for x={x}"))
}

fn run_bb(
    sut_mutant: bool,
    ctx: &mut noprop::TestCaseContext,
    _obs: &Observe,
) -> Result<(), String> {
    let x = crate::bb::u32(ctx);
    process(x, sut_mutant).map_err(|_| format!("process failed for x={x}"))
}

fn run_domain(
    sut_mutant: bool,
    biased: bool,
    ctx: &mut noprop::TestCaseContext,
    _obs: &Observe,
) -> Result<(), String> {
    let x = if biased {
        // 10% the domain value, 90% uniform.
        noprop::sample_with_boundaries(ctx, &[1500], noprop::Ratio::one_nth(10), noprop::sample_u32)
    } else {
        noprop::sample_u32(ctx)
    };
    process_domain(x, sut_mutant).map_err(|_| format!("process failed for x={x}"))
}

fn run_domain_bb(
    sut_mutant: bool,
    ctx: &mut noprop::TestCaseContext,
    _obs: &Observe,
) -> Result<(), String> {
    let x = crate::bb::u32(ctx);
    process_domain(x, sut_mutant).map_err(|_| format!("process failed for x={x}"))
}

fn run_range(
    sut_mutant: bool,
    biased: bool,
    ctx: &mut noprop::TestCaseContext,
    _obs: &Observe,
) -> Result<(), String> {
    let v = if biased {
        // 10% the range end, 90% the rest of the range.
        noprop::sample_with_boundaries(ctx, &[1023], noprop::Ratio::one_nth(10), |c| {
            noprop::sample_usize_in(c, 0..1023)
        })
    } else {
        noprop::sample_usize_in(ctx, 0..1024)
    };
    process_range(v, sut_mutant).map_err(|_| format!("process failed for v={v}"))
}

fn run_range_bb(
    sut_mutant: bool,
    ctx: &mut noprop::TestCaseContext,
    _obs: &Observe,
) -> Result<(), String> {
    // The bounded range draw is not wrapped by the generic mix, so
    // this generator is identical to the uniform one.
    let v = noprop::sample_usize_in(ctx, 0..1024);
    process_range(v, sut_mutant).map_err(|_| format!("process failed for v={v}"))
}

pub(crate) const WORKLOAD: Workload = Workload {
    name: "boundary",
    description: "mutants fail only for boundary values of three kinds",
    tasks: &[
        Task {
            mutant: "fails_on_zero",
            base: run_base,
            uniform: run_uniform,
            biased: run_biased,
            bb: run_bb_uniform,
        },
        Task {
            mutant: "fails_on_domain_value",
            base: run_domain_base,
            uniform: run_domain_uniform,
            biased: run_domain_biased,
            bb: run_domain_bb_uniform,
        },
        Task {
            mutant: "fails_on_range_end",
            base: run_range_base,
            uniform: run_range_uniform,
            biased: run_range_biased,
            bb: run_range_bb_uniform,
        },
    ],
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
fn run_bb_uniform(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_bb(true, ctx, obs)
}

fn run_domain_base(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_domain(false, false, ctx, obs)
}
fn run_domain_uniform(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_domain(true, false, ctx, obs)
}
fn run_domain_biased(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_domain(true, true, ctx, obs)
}
fn run_domain_bb_uniform(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_domain_bb(true, ctx, obs)
}

fn run_range_base(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_range(false, false, ctx, obs)
}
fn run_range_uniform(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_range(true, false, ctx, obs)
}
fn run_range_biased(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_range(true, true, ctx, obs)
}
fn run_range_bb_uniform(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run_range_bb(true, ctx, obs)
}
