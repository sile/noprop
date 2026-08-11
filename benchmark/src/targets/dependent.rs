//! Dependent-structure target: a shrunken MP4 `trun` box where the
//! `flags` bits decide which optional fields follow. The mutant
//! misreads the duration field when both field bits are set, so only
//! inputs whose flags are `0b11` expose it.
//!
//! This workload also reports semantic-bucket observations: the count
//! of each flag value generated. A run stops at the first failure, so
//! on a detected trial the counts cover only the cases up to
//! detection; the base variant (which completes every trial) is the
//! one that shows the generator's full breadth.

use super::{Observe, Task, Workload};

/// Shrunken `trun` box: bit 0 of `flags` means a duration field
/// follows, bit 1 means a size field follows.
///
/// Returns the parsed duration (or `None` when the field is absent).
/// Under the mutant, the duration field is misread by one when both
/// bits are set.
fn parse_trun(flags: u8, duration: u32, size: u32, mutant: bool) -> (Option<u32>, Option<u32>) {
    let parsed_duration = if flags & 0b01 != 0 {
        let mut value = duration;
        if mutant && flags & 0b10 != 0 {
            value = value.wrapping_add(1);
        }
        Some(value)
    } else {
        None
    };
    let parsed_size = if flags & 0b10 != 0 { Some(size) } else { None };
    (parsed_duration, parsed_size)
}

fn run(
    sut_mutant: bool,
    biased: bool,
    bb: bool,
    ctx: &mut noprop::TestCaseContext,
    obs: &Observe,
) -> Result<(), String> {
    let flags = if biased {
        // 90% both bits set, otherwise uniform over the four values.
        if noprop::sample_usize_in(ctx, 0..10) < 9 {
            0b11
        } else {
            noprop::sample_usize_in(ctx, 0..4) as u8
        }
    } else {
        noprop::sample_usize_in(ctx, 0..4) as u8
    };
    obs.add(
        match flags {
            0 => "flag_0",
            1 => "flag_1",
            2 => "flag_2",
            _ => "flag_3",
        },
        1,
    );

    // Feedback: the mutant misreads the duration when both bits are
    // set, so the bucket observes the flag distribution. This call
    // draws no random bytes, so the generator stream is unchanged.
    ctx.bucket("flags", flags as u64);

    let duration = if bb {
        crate::bb::u32(ctx)
    } else {
        noprop::sample_u32(ctx)
    };
    let size = if bb {
        crate::bb::u32(ctx)
    } else {
        noprop::sample_u32(ctx)
    };
    let parsed_duration = parse_trun(flags, duration, size, sut_mutant).0;

    // Property: the parsed duration must agree with the flags and the
    // generated value. The mutant's misread makes the duration
    // disagree whenever both bits are set. (The size field is passed
    // through unchanged by both the base SUT and the mutant.)
    let expected_duration = (flags & 0b01 != 0).then_some(duration);
    if parsed_duration != expected_duration {
        return Err(format!(
            "duration field misread for flags={flags}: parsed {parsed_duration:?}, expected {expected_duration:?}"
        ));
    }
    Ok(())
}

pub(crate) const WORKLOAD: Workload = Workload {
    name: "dependent",
    description: "shrunken MP4 trun box; field presence depends on flags",
    tasks: &[Task {
        mutant: "duration_field_misread",
        base: run_base,
        uniform: run_uniform,
        biased: run_biased,
        bb: run_bb,
    }],
};

fn run_base(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(false, false, false, ctx, obs)
}
fn run_uniform(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(true, false, false, ctx, obs)
}
fn run_biased(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(true, true, false, ctx, obs)
}
fn run_bb(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(true, false, true, ctx, obs)
}
