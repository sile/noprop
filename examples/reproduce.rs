//! Reproducing a failure: a failing seed is the unit of debugging.
//! The failure report prints a `reproduce with:` hint, and re-running
//! the same seed hits the identical failure case.
//!
//! The seed is read from `NOPROP_SEED` (falling back to a fixed
//! default), so the workflow is: run, copy the reported seed into
//! `NOPROP_SEED`, re-run — same failure, same case index.
//!
//! This example fails on purpose — a non-zero exit code is the
//! expected behavior (CI verifies it).

/// A property whose failure is value-dependent: roughly half of the
/// draws violate the assertion, so the failing case index is decided
/// by the seed.
fn run(seed: u64) -> noprop::RunResult {
    noprop::Runner::new(seed).run(64, |ctx| {
        let x = noprop::sample_u32(ctx);
        assert!(x < 0x8000_0000, "high bit set: {x:#010x}");
        Ok(())
    })
}

/// Parse a seed from the environment: accepts decimal and `0x`-prefixed
/// hex with optional `_` separators, matching the format printed by
/// the failure report.
fn parse_seed(s: &str) -> Option<u64> {
    let cleaned: String = s.chars().filter(|c| *c != '_').collect();
    if let Some(hex) = cleaned.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()
    } else {
        cleaned.parse().ok()
    }
}

fn main() {
    // `NOPROP_SEED` overrides; the fixed default keeps the demo
    // deterministic without setup (and the CI expectation stable).
    let seed = match std::env::var("NOPROP_SEED") {
        Ok(s) => parse_seed(&s).expect("NOPROP_SEED must be decimal or 0x-prefixed hex"),
        // Any fixed seed works; this one fails at a non-trivial case
        // index (printed below) so the demo shows a non-zero index.
        Err(_) => 0x00FF_00FF_00FF_00FF,
    };
    let err = run(seed).expect_err("this seed must fail");

    eprintln!("--- first run ---");
    eprintln!("{err}");
    let first_case = err.case_index();

    // Re-run with the seed from the report: the same failure case
    // must be hit again.
    let replay = run(err.seed()).expect_err("the same seed must fail again");
    eprintln!("--- replay ---");
    eprintln!("{replay}");
    assert_eq!(
        first_case,
        replay.case_index(),
        "the replay must fail at the same case index"
    );
    println!("reproduced: seed {seed:#018x} fails at case {first_case} on every run");
    println!("to reproduce this failure outside this example:");
    println!("  NOPROP_SEED={seed:#018x} cargo run --example reproduce");

    // Deliberate failure: this example demonstrates the failure
    // report, so exiting non-zero is the expected outcome.
    std::process::exit(1);
}
