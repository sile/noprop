//! One-time proptest comparison harness (debug branch only).
//!
//! Reimplements the detection-benchmark workloads as proptest
//! strategies and runs them over a seed cohort with shrinking
//! disabled, emitting the same raw-result JSON lines as the noprop
//! harness so detection rates and iterations-to-detection are
//! directly comparable.
//!
//! The SUT/property logic is duplicated from the noprop harness on
//! purpose: this example is a throwaway comparison tool and is never
//! merged into main.

use std::cell::Cell;
use std::time::Instant;

use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed, TestCaseError, TestRunner};

// ---------------------------------------------------------------------------
// SUTs and properties (duplicated from examples/detection_benchmark/targets)
// ---------------------------------------------------------------------------

fn high_frequency_process(x: u32, mutant: bool) -> Result<(), String> {
    if mutant && !x.is_multiple_of(2) {
        return Err(format!("process failed for x={x}"));
    }
    Ok(())
}

fn boundary_process(x: u32, mutant: bool) -> Result<(), String> {
    if mutant && x == 0 {
        return Err(format!("process failed for x={x}"));
    }
    Ok(())
}

fn combination_process(x: u32, y: u32, mutant: bool) -> Result<(), String> {
    if mutant && x == 1 && y == 2 {
        return Err(format!("process failed for ({x}, {y})"));
    }
    Ok(())
}

fn dependent_parse(flags: u8, duration: u32, size: u32, mutant: bool) -> Result<(), String> {
    let parsed_duration = if flags & 0b01 != 0 {
        let mut value = duration;
        if mutant && flags & 0b10 != 0 {
            value = value.wrapping_add(1);
        }
        Some(value)
    } else {
        None
    };
    let expected_duration = (flags & 0b01 != 0).then_some(duration);
    if parsed_duration != expected_duration {
        return Err(format!(
            "duration field misread for flags={flags}: parsed {parsed_duration:?}, expected {expected_duration:?}"
        ));
    }
    let _ = size;
    Ok(())
}

/// Minimal binary search tree used as the SUT (from the bst workload).
#[derive(Default)]
struct Bst {
    root: Option<Box<Node>>,
}

struct Node {
    key: u32,
    left: Option<Box<Node>>,
    right: Option<Box<Node>>,
}

impl Bst {
    fn insert(&mut self, key: u32, duplicate_bug: bool) {
        if !duplicate_bug && self.contains(key) {
            return;
        }
        self.root = insert_into(self.root.take(), key);
    }

    fn contains(&self, key: u32) -> bool {
        let mut node = self.root.as_deref();
        while let Some(n) = node {
            if key == n.key {
                return true;
            }
            node = if key < n.key { n.left.as_deref() } else { n.right.as_deref() };
        }
        false
    }

    fn delete(&mut self, key: u32) {
        self.root = delete_from(self.root.take(), key);
    }

    fn keys(&self) -> Vec<u32> {
        let mut out = Vec::new();
        collect(&self.root, &mut out);
        out
    }
}

fn insert_into(node: Option<Box<Node>>, key: u32) -> Option<Box<Node>> {
    let Some(mut node) = node else {
        return Some(Box::new(Node {
            key,
            left: None,
            right: None,
        }));
    };
    if key < node.key {
        node.left = insert_into(node.left.take(), key);
    } else {
        node.right = insert_into(node.right.take(), key);
    }
    Some(node)
}

fn delete_from(node: Option<Box<Node>>, key: u32) -> Option<Box<Node>> {
    let mut node = node?;
    if key < node.key {
        node.left = delete_from(node.left.take(), key);
        return Some(node);
    }
    if key > node.key {
        node.right = delete_from(node.right.take(), key);
        return Some(node);
    }
    match (node.left.take(), node.right.take()) {
        (None, right) => right,
        (left, None) => left,
        (left, Some(right)) => {
            let successor_key = successor_key(&right);
            let right = delete_from(Some(right), successor_key);
            Some(Box::new(Node {
                key: successor_key,
                left,
                right,
            }))
        }
    }
}

fn successor_key(node: &Node) -> u32 {
    let mut node = node;
    while let Some(next) = node.left.as_deref() {
        node = next;
    }
    node.key
}

fn collect(node: &Option<Box<Node>>, out: &mut Vec<u32>) {
    if let Some(node) = node {
        collect(&node.left, out);
        out.push(node.key);
        collect(&node.right, out);
    }
}

fn bst_check(ops: &[(bool, u32)], mutant: bool) -> Result<(), String> {
    let mut bst = Bst::default();
    let mut model = std::collections::BTreeMap::new();
    for &(insert, key) in ops {
        if insert {
            bst.insert(key, mutant);
            model.insert(key, ());
        } else {
            bst.delete(key);
            model.remove(&key);
        }
    }
    let actual = bst.keys();
    let expected: Vec<u32> = model.keys().copied().collect();
    if actual != expected {
        return Err(format!("bst keys {actual:?} != model keys {expected:?}"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Proptest strategies for each workload
// ---------------------------------------------------------------------------

fn high_frequency_strategy(biased: bool) -> BoxedStrategy<u32> {
    if biased {
        // 90% odd, 10% even (mirrors the noprop biased variant).
        prop_oneof![
            9 => any::<u32>().prop_map(|v| v | 1),
            1 => any::<u32>().prop_map(|v| v & !1),
        ]
        .boxed()
    } else {
        any::<u32>().boxed()
    }
}

fn boundary_strategy(biased: bool) -> BoxedStrategy<u32> {
    if biased {
        // 10% exactly zero.
        prop_oneof![1 => Just(0u32), 9 => any::<u32>()].boxed()
    } else {
        any::<u32>().boxed()
    }
}

fn combination_strategy(biased: bool) -> BoxedStrategy<(u32, u32)> {
    if biased {
        // x: 50% witness 1, y: 50% witness 2.
        let x = prop_oneof![1 => Just(1u32), 1 => any::<u32>()];
        let y = prop_oneof![1 => Just(2u32), 1 => any::<u32>()];
        (x, y).boxed()
    } else {
        (any::<u32>(), any::<u32>()).boxed()
    }
}

fn dependent_strategy(biased: bool) -> BoxedStrategy<(u8, u32, u32)> {
    let flags: BoxedStrategy<u8> = if biased {
        prop_oneof![9 => Just(0b11u8), 1 => 0u8..4].boxed()
    } else {
        (0u8..4).boxed()
    };
    (flags, any::<u32>(), any::<u32>()).boxed()
}

/// Operation sequence as (len, ops); only the first `len` ops are used,
/// mirroring the noprop generator's `max_ops` draw.
fn bst_strategy(biased: bool) -> BoxedStrategy<(usize, Vec<(bool, u32)>)> {
    let op: BoxedStrategy<(bool, u32)> = if biased {
        let key = prop_oneof![1 => 0u32..8, 1 => 0u32..1024];
        (prop_oneof![8 => Just(true), 2 => Just(false)], key).boxed()
    } else {
        (any::<bool>(), 0u32..1024).boxed()
    };
    (0usize..16, prop::collection::vec(op, 16)).boxed()
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct TaskResult {
    status: &'static str,
    detected_at: Option<usize>,
    accepted_iterations: usize,
    total_samples: usize,
    wall_clock_ns: u128,
}

/// Run one task over a strategy, mirroring the noprop harness output.
fn run_task<S: Strategy>(
    strategy: &S,
    test: &impl Fn(&S::Value) -> Result<(), String>,
    seed: u64,
    iterations: usize,
) -> TaskResult {
    let mut runner = TestRunner::new(Config {
        cases: iterations as u32,
        failure_persistence: None,
        rng_seed: RngSeed::Fixed(seed),
        ..Config::default()
    });

    let case = Cell::new(0usize);
    let detected_at = Cell::new(None);
    let start = Instant::now();
    let outcome = runner.run(&strategy.no_shrink(), |value| {
        let idx = case.get() + 1;
        case.set(idx);
        match test(&value) {
            Ok(()) => Ok(()),
            Err(_) => {
                detected_at.set(Some(idx));
                Err(TestCaseError::fail("property failed"))
            }
        }
    });
    let wall_clock_ns = start.elapsed().as_nanos();
    let detected = detected_at.get();
    let status = match outcome {
        Ok(()) => "not_found",
        Err(_) if detected.is_some() => "found",
        Err(_) => "gave_up",
    };
    TaskResult {
        status,
        detected_at: detected,
        accepted_iterations: detected.unwrap_or(iterations),
        total_samples: detected.unwrap_or(iterations),
        wall_clock_ns,
    }
}

fn emit(
    workload: &str,
    mutant: &str,
    variant: &str,
    seed: u64,
    iterations: usize,
    result: &TaskResult,
) {
    let json = nojson::object(|f| {
        f.member("format_version", 1u32)?;
        f.member("workload", workload)?;
        f.member("mutant", mutant)?;
        f.member("variant", variant)?;
        f.member("seed", seed)?;
        f.member("iterations", iterations)?;
        f.member("status", result.status)?;
        f.member("detected_at", result.detected_at)?;
        f.member("accepted_iterations", result.accepted_iterations)?;
        f.member("rejected_iterations", 0u32)?;
        f.member("total_samples", result.total_samples)?;
        f.member("observations", nojson::array(|_| Ok(())))?;
        f.member("wall_clock_ns", result.wall_clock_ns)
    });
    println!("{}", nojson::Json(&json));
}

fn main() {
    let mut iterations = 1000usize;
    let mut seeds: Vec<u64> = (0..64).collect();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--iterations" => {
                iterations = args[i + 1].parse().expect("invalid --iterations");
                i += 2;
            }
            "--seeds" => {
                seeds = args[i + 1]
                    .split(',')
                    .map(|s| s.trim().parse().expect("invalid seed"))
                    .collect();
                i += 2;
            }
            other => panic!("unknown argument {other:?}"),
        }
    }

    for &seed in &seeds {
        // high-frequency
        run_emit(
            "high-frequency",
            "fails_on_odd",
            "proptest-uniform",
            &high_frequency_strategy(false),
            &|x| high_frequency_process(*x, true),
            seed,
            iterations,
        );
        run_emit(
            "high-frequency",
            "fails_on_odd",
            "proptest-biased",
            &high_frequency_strategy(true),
            &|x| high_frequency_process(*x, true),
            seed,
            iterations,
        );
        // boundary
        run_emit(
            "boundary",
            "fails_on_zero",
            "proptest-uniform",
            &boundary_strategy(false),
            &|x| boundary_process(*x, true),
            seed,
            iterations,
        );
        run_emit(
            "boundary",
            "fails_on_zero",
            "proptest-biased",
            &boundary_strategy(true),
            &|x| boundary_process(*x, true),
            seed,
            iterations,
        );
        // combination
        run_emit(
            "combination",
            "fails_on_specific_pair",
            "proptest-uniform",
            &combination_strategy(false),
            &|(x, y)| combination_process(*x, *y, true),
            seed,
            iterations,
        );
        run_emit(
            "combination",
            "fails_on_specific_pair",
            "proptest-biased",
            &combination_strategy(true),
            &|(x, y)| combination_process(*x, *y, true),
            seed,
            iterations,
        );
        // dependent
        run_emit(
            "dependent",
            "duration_field_misread",
            "proptest-uniform",
            &dependent_strategy(false),
            &|(flags, duration, size)| dependent_parse(*flags, *duration, *size, true),
            seed,
            iterations,
        );
        run_emit(
            "dependent",
            "duration_field_misread",
            "proptest-biased",
            &dependent_strategy(true),
            &|(flags, duration, size)| dependent_parse(*flags, *duration, *size, true),
            seed,
            iterations,
        );
        // bst
        run_emit(
            "bst",
            "insert_duplicate_key",
            "proptest-uniform",
            &bst_strategy(false),
            &|(len, ops)| bst_check(&ops[..*len], true),
            seed,
            iterations,
        );
        run_emit(
            "bst",
            "insert_duplicate_key",
            "proptest-biased",
            &bst_strategy(true),
            &|(len, ops)| bst_check(&ops[..*len], true),
            seed,
            iterations,
        );
    }
}

fn run_emit<S: Strategy>(
    workload: &str,
    mutant: &str,
    variant: &str,
    strategy: &S,
    test: &impl Fn(&S::Value) -> Result<(), String>,
    seed: u64,
    iterations: usize,
) {
    let result = run_task(strategy, test, seed, iterations);
    emit(workload, mutant, variant, seed, iterations, &result);
}
