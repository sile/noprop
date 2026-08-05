//! Smoke tests for the detection benchmark harness.
//!
//! These exercise the harness as a subprocess (`CARGO_BIN_EXE_...`):
//! determinism, ground truth (base completes, mutants are detected by
//! known witnesses), and summary regeneration from raw results.

use std::process::Command;

/// Workload / mutant pairs registered by the harness.
const TASKS: &[(&str, &str)] = &[
    ("high-frequency", "fails_on_odd"),
    ("boundary", "fails_on_zero"),
    ("combination", "fails_on_specific_pair"),
    ("dependent", "duration_field_misread"),
    ("bst", "insert_duplicate_key"),
];

fn run(args: &[&str]) -> String {
    run_with_stdin(args, None)
}

fn run_with_stdin(args: &[&str], stdin: Option<&str>) -> String {
    use std::io::Write;
    // `cargo test` builds the examples before running integration
    // tests, so the harness binary is always present at this path.
    let binary = format!(
        "{}/target/debug/examples/detection_benchmark",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut child = Command::new(binary)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("failed to run the detection benchmark");
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(input.as_bytes())
            .expect("failed to write stdin");
    }
    let output = child.wait_with_output().expect("failed to wait");
    assert!(
        output.status.success(),
        "command failed ({args:?}): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("non-UTF-8 output")
}

/// Drop the `wall_clock_ns` field (the last field of a raw result),
/// which is timing noise and not reproducible.
fn strip_wall_clock(line: &str) -> String {
    match line.rfind(",\"wall_clock_ns\":") {
        Some(pos) => line[..pos].to_string(),
        None => line.to_string(),
    }
}

fn run_task(workload: &str, mutant: &str, variant: &str, seed: u64) -> String {
    run(&[
        "run",
        "--workload",
        workload,
        "--mutant",
        mutant,
        "--variant",
        variant,
        "--seed",
        &seed.to_string(),
        "--iterations",
        "100",
    ])
}

/// The base SUT must complete the property for any generated input:
/// every base run ends `not_found`.
#[test]
fn base_sut_completes_for_all_workloads() {
    for (workload, mutant) in TASKS {
        let line = run_task(workload, mutant, "base", 1);
        assert!(
            line.contains("\"status\":\"not_found\""),
            "base SUT of {workload}/{mutant} must complete: {line}"
        );
    }
}

/// The mutant must be detected by a known witness: the biased variant
/// steers toward the witness and reports `found`.
#[test]
fn mutants_are_detected_by_biased_generation() {
    for (workload, mutant) in TASKS {
        let line = run_task(workload, mutant, "biased", 1);
        assert!(
            line.contains("\"status\":\"found\""),
            "mutant {workload}/{mutant} must be detected by the biased variant: {line}"
        );
    }
}

/// Same seed and arguments must reproduce the identical raw result
/// (the wall-clock field is timing noise and excluded).
#[test]
fn same_seed_is_deterministic() {
    for (workload, mutant) in TASKS {
        let a = strip_wall_clock(&run_task(workload, mutant, "uniform", 42));
        let b = strip_wall_clock(&run_task(workload, mutant, "uniform", 42));
        assert_eq!(
            a, b,
            "{workload}/{mutant} must be reproducible from the seed"
        );
    }
}

/// The summary subcommand must aggregate every raw-result line without
/// dropping any task group.
#[test]
fn summary_regenerates_from_raw_results() {
    let seeds = "0,1,2";
    let raw = run(&["run-all", "--iterations", "50", "--seeds", seeds]);
    let groups = run_with_stdin(&["summary"], Some(&raw));
    // run-all prints one JSON line per (workload, mutant, variant, seed);
    // summary prints one line per (variant, workload, mutant) group.
    let raw_lines = raw.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(
        raw_lines,
        TASKS.len() * 2 * 3,
        "raw lines must match tasks x variants x seeds"
    );
    let expected_groups = TASKS.len() * 2;
    let summary_lines = groups.lines().filter(|l| l.starts_with("variant=")).count();
    assert_eq!(
        summary_lines, expected_groups,
        "summary must cover every (variant, workload, mutant) group"
    );
}
