//! Reproducing a failure: a failing seed is the unit of debugging.
//! The failure report prints a `reproduce with:` hint, and re-running
//! the same seed hits the identical failure case.
//!
//! This example fails on purpose — a non-zero exit code is the
//! expected behavior (CI verifies it).

fn run(seed: u64) -> noprop::RunResult {
    let case = std::cell::Cell::new(0usize);
    noprop::Runner::new(seed).run(64, |ctx| {
        let n = case.get();
        case.set(n + 1);
        let _ = noprop::sample_u32(ctx);
        if n >= 3 {
            panic!("boom at case {n}");
        }
        Ok(())
    })
}

fn main() {
    // A seed whose failure lands at a non-zero case index.
    let seed = 0xBAD_CAFE_1234_5678;
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

    // Deliberate failure: this example demonstrates the failure
    // report, so exiting non-zero is the expected outcome.
    std::process::exit(1);
}
