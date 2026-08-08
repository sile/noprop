//! Smoke tests for the detection benchmark harness.
//!
//! These exercise the harness as a subprocess (`CARGO_BIN_EXE_...`):
//! determinism, ground truth (base completes, mutants are detected by
//! known witnesses), and summary regeneration from raw results.

use std::process::Command;
use std::sync::OnceLock;

/// Build the harness once per test process: a filtered
/// `cargo test --test ...` run does not build examples, and running
/// `cargo build` on every invocation would race between the parallel
/// tests while the binary is being replaced.
static BUILD: OnceLock<()> = OnceLock::new();

fn ensure_built() {
    BUILD.get_or_init(|| {
        let build = Command::new(env!("CARGO"))
            .args(["build", "--example", "detection_benchmark"])
            .status()
            .expect("failed to run cargo build");
        assert!(
            build.success(),
            "cargo build --example detection_benchmark failed"
        );
    });
}

/// Workload / mutant pairs registered by the harness, excluding the
/// guard workload (which has no mutant and never detects).
const TASKS: &[(&str, &str)] = &[
    ("high-frequency", "fails_on_odd"),
    ("boundary", "fails_on_zero"),
    ("boundary", "fails_on_domain_value"),
    ("boundary", "fails_on_range_end"),
    ("combination", "fails_on_specific_pair"),
    ("dependent", "duration_field_misread"),
    ("bst", "insert_duplicate_key"),
    ("stepping", "fails_on_five_zeros"),
    ("stateful", "fails_on_state_seven"),
];

/// Workloads with no mutant: detection is never expected.
const GUARD_TASKS: &[(&str, &str)] = &[("guard", "reports_unbounded_buckets")];

/// Comparison variants run by `run-all`.
const VARIANTS: &[&str] = &["uniform", "biased", "boundary-biased", "corpus-guided"];

fn run(args: &[&str]) -> String {
    run_with_stdin(args, None)
}

fn run_with_stdin(args: &[&str], stdin: Option<&str>) -> String {
    use std::io::Write;
    ensure_built();
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
        "--cases",
        "100",
    ])
}

/// The base SUT must complete the property for any generated input:
/// every base run ends `not_found`.
#[test]
fn base_sut_completes_for_all_workloads() {
    for (workload, mutant) in TASKS.iter().chain(GUARD_TASKS) {
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

/// The guard workload must never report a detection: it has no mutant.
#[test]
fn guard_never_detects() {
    for (workload, mutant) in GUARD_TASKS {
        let line = run_task(workload, mutant, "uniform", 1);
        assert!(
            line.contains("\"status\":\"not_found\""),
            "guard {workload}/{mutant} must complete without detection: {line}"
        );
    }
}

/// Same seed and arguments must reproduce the identical raw result
/// (the wall-clock field is timing noise and excluded), for every
/// search variant.
#[test]
fn same_seed_is_deterministic() {
    for (workload, mutant) in TASKS.iter().chain(GUARD_TASKS) {
        for variant in VARIANTS {
            let a = strip_wall_clock(&run_task(workload, mutant, variant, 42));
            let b = strip_wall_clock(&run_task(workload, mutant, variant, 42));
            assert_eq!(
                a, b,
                "{workload}/{mutant} under {variant} must be reproducible from the seed"
            );
        }
    }
}

/// Every variant must complete the guard workload without aborting
/// (a property failure would surface as `found`).
#[test]
fn guard_completes_under_every_variant() {
    for (workload, mutant) in GUARD_TASKS {
        for variant in VARIANTS {
            let line = run_task(workload, mutant, variant, 1);
            assert!(
                line.contains("\"status\":\"not_found\""),
                "guard {workload}/{mutant} under {variant} must complete: {line}"
            );
        }
    }
}

/// The summary subcommand must aggregate every raw-result line without
/// dropping any task group.
#[test]
fn summary_regenerates_from_raw_results() {
    let seeds = "0,1,2";
    let raw = run(&["run-all", "--cases", "50", "--seeds", seeds]);
    let groups = run_with_stdin(&["summary"], Some(&raw));
    // run-all prints one JSON line per (workload, mutant, variant, seed);
    // summary prints one line per (variant, workload, mutant) group.
    let task_count = TASKS.len() + GUARD_TASKS.len();
    let raw_lines = raw.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(
        raw_lines,
        task_count * VARIANTS.len() * 3,
        "raw lines must match tasks x variants x seeds"
    );
    let expected_groups = task_count * VARIANTS.len();
    let summary_lines = groups.lines().filter(|l| l.starts_with("variant=")).count();
    assert_eq!(
        summary_lines, expected_groups,
        "summary must cover every (variant, workload, mutant) group"
    );
    // Every group must aggregate all three seeds: a dropped raw line
    // would shrink `trials` while keeping the group count intact.
    for line in groups.lines().filter(|l| l.starts_with("variant=")) {
        assert!(
            line.contains("trials=3"),
            "every group must aggregate all seeds: {line}"
        );
    }
}
