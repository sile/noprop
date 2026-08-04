//! Compare uniform sampling with targeted search on rare failures.
//!
//! Case A: a single bounded draw fails at the top 1 % of its domain.
//! Case B: two bounded draws fail only when both land in the top 1 %
//! (a 0.01 % failure rate), so the search must explore combinations.
//!
//! Observed behavior with 32 seeds (noprop 0.0.1, initial search
//! constants):
//! - Case A: uniform detects more failures. Bounded-domain mutation
//!   rewrites a draw to a fresh random value inside the domain, so on
//!   a single draw the targeted corpus adds little over uniform
//!   sampling.
//! - Case B: targeted detects about three times as many failures. The
//!   corpus keeps high-scoring *combinations* of draws, and mutation
//!   varies one draw while the other stays favorable.
//!
//! The case-A gap is a known limitation of the initial mutation
//! strategy; its tuning is evaluated by the detection benchmark.

use noprop::TestCaseContext;

fn case_a(ctx: &mut TestCaseContext) -> Result<(), Box<dyn std::error::Error>> {
    let x = noprop::sample_usize_in(ctx, 0..1000);
    if x >= 990 {
        panic!("a: x = {x}");
    }
    ctx.maximize(x as f64 / 1000.0);
    Ok(())
}

fn case_b(ctx: &mut TestCaseContext) -> Result<(), Box<dyn std::error::Error>> {
    let a = noprop::sample_usize_in(ctx, 0..1000);
    let b = noprop::sample_usize_in(ctx, 0..1000);
    if a >= 990 && b >= 990 {
        panic!("b: a = {a}, b = {b}");
    }
    ctx.maximize((a as f64 + b as f64) / 2000.0);
    Ok(())
}

fn mean(xs: &[usize]) -> f64 {
    xs.iter().sum::<usize>() as f64 / xs.len() as f64
}

fn compare(
    name: &str,
    property: fn(&mut TestCaseContext) -> Result<(), Box<dyn std::error::Error>>,
    iterations: usize,
) {
    const SEEDS: usize = 32;
    let mut uniform_hits = 0usize;
    let mut uniform_detection: Vec<usize> = Vec::new();
    let mut targeted_hits = 0usize;
    let mut targeted_detection: Vec<usize> = Vec::new();

    for seed in 0..SEEDS {
        let mut runner = noprop::Runner::new(seed as u64, iterations);
        if let Err(err) = runner.run(property) {
            uniform_hits += 1;
            uniform_detection.push(err.case_index() + 1);
        }
        let mut runner = noprop::Runner::new(seed as u64, iterations);
        if let Err(err) = runner.run_targeted(property) {
            targeted_hits += 1;
            targeted_detection.push(err.case_index() + 1);
        }
    }

    let fmt_mean = |xs: &[usize]| -> String {
        if xs.is_empty() {
            "no detection".to_string()
        } else {
            format!("{:.1}", mean(xs))
        }
    };

    println!("[{name}] seeds: {SEEDS}, iterations per run: {iterations}");
    println!(
        "  uniform : {uniform_hits} hits, mean iterations-to-detection = {}",
        fmt_mean(&uniform_detection)
    );
    println!(
        "  targeted: {targeted_hits} hits, mean iterations-to-detection = {}",
        fmt_mean(&targeted_detection)
    );
}

fn main() {
    // Silence the default panic hook: the demo reports failures via
    // case_index, and the hook would bury the summary in panic output.
    std::panic::set_hook(Box::new(|_| {}));
    compare("A: single draw, 1% failure", case_a, 200);
    compare("B: two draws, 0.01% failure", case_b, 2000);
}
