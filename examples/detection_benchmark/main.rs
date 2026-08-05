//! Detection benchmark harness.
//!
//! Subcommands:
//!
//! - `run`: run a single task (workload x mutant x variant x seed) and
//!   print one raw-result JSON line.
//! - `run-all`: run every task over a seed cohort and print one
//!   raw-result JSON line per trial.
//! - `summary`: read raw-result JSON lines from stdin and print a
//!   per-task bucket summary (regenerated from the raw results).

mod raw;
mod summary;
mod targets;
mod variants;

use noargs::{Error, cmd, opt, raw_args};

fn main() -> Result<(), Error> {
    let mut args = raw_args();
    args.metadata_mut().app_name = "noprop-detection-benchmark";
    args.metadata_mut().app_description =
        "Measure detection rate and iterations-to-detection of noprop generators.";

    if noargs::VERSION_FLAG.take(&mut args).is_present() {
        println!("noprop-detection-benchmark {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    noargs::HELP_FLAG.take_help(&mut args);

    if run_cmd(&mut args)? {
        return Ok(());
    }
    if run_all_cmd(&mut args)? {
        return Ok(());
    }
    if summary_cmd(&mut args)? {
        return Ok(());
    }

    if let Some(help) = args.finish()? {
        print!("{help}");
    }
    Ok(())
}

fn run_cmd(args: &mut noargs::RawArgs) -> Result<bool, Error> {
    if !cmd("run")
        .doc("Run a single task and print one raw-result JSON line")
        .take(args)
        .is_present()
    {
        return Ok(false);
    }

    let workload_name: String = opt("workload")
        .ty("NAME")
        .doc("Workload name")
        .take(args)
        .then(|o| o.value().parse())?;
    let mutant_name: String = opt("mutant")
        .ty("NAME")
        .doc("Mutant name")
        .take(args)
        .then(|o| o.value().parse())?;
    let variant_name: String = opt("variant")
        .ty("NAME")
        .doc("Generator variant: uniform | biased (or base for the ground-truth SUT)")
        .take(args)
        .then(|o| o.value().parse())?;
    let seed: u64 = opt("seed")
        .ty("N")
        .doc("Seed for the run")
        .take(args)
        .then(|o| o.value().parse())?;
    let iterations: usize = opt("iterations")
        .ty("N")
        .doc("Accepted iteration budget")
        .default("1000")
        .take(args)
        .then(|o| o.value().parse())?;

    let workload = targets::workload(&workload_name).ok_or_else(|| {
        let available = targets::WORKLOADS
            .iter()
            .map(|w| format!("{} ({})", w.name, w.description))
            .collect::<Vec<_>>()
            .join(", ");
        Error::other(
            args,
            format!("unknown workload {workload_name:?}; available: {available}"),
        )
    })?;
    let task = targets::task(workload, &mutant_name)
        .ok_or_else(|| Error::other(args, format!("unknown mutant {mutant_name:?}")))?;
    let variant = variants::Variant::from_str(&variant_name)
        .ok_or_else(|| Error::other(args, format!("unknown variant {variant_name:?}")))?;

    let result = variants::run_task(workload.name, task, variant, seed, iterations);
    println!("{}", nojson::Json(&result));
    Ok(true)
}

fn run_all_cmd(args: &mut noargs::RawArgs) -> Result<bool, Error> {
    if !cmd("run-all")
        .doc("Run every task over a seed cohort and print raw-result JSON lines")
        .take(args)
        .is_present()
    {
        return Ok(false);
    }

    let iterations: usize = opt("iterations")
        .ty("N")
        .doc("Accepted iteration budget")
        .default("1000")
        .take(args)
        .then(|o| o.value().parse())?;
    let seeds_text: String = opt("seeds")
        .ty("LIST")
        .doc("Comma-separated seed list, e.g. 0,1,2,3")
        .take(args)
        .then(|o| o.value().parse())?;
    let seeds: Vec<u64> = seeds_text
        .split(',')
        .map(|s| s.trim().parse::<u64>())
        .collect::<Result<_, _>>()
        .map_err(|e| Error::other(args, format!("invalid seed list {seeds_text:?}: {e}")))?;
    if seeds.is_empty() {
        return Err(Error::other(args, "seed list must not be empty"));
    }

    for workload in targets::WORKLOADS {
        for task in workload.tasks {
            for variant in variants::VARIANTS {
                for &seed in &seeds {
                    let result =
                        variants::run_task(workload.name, task, *variant, seed, iterations);
                    println!("{}", nojson::Json(&result));
                }
            }
        }
    }
    Ok(true)
}

fn summary_cmd(args: &mut noargs::RawArgs) -> Result<bool, Error> {
    if !cmd("summary")
        .doc("Read raw-result JSON lines from stdin and print a bucket summary")
        .take(args)
        .is_present()
    {
        return Ok(false);
    }

    let stdin = std::io::stdin();
    let (summaries, skipped) = summary::read_summaries(stdin.lock());
    print_summary(&summaries);
    if skipped > 0 {
        eprintln!("warning: skipped {skipped} malformed raw-result line(s)");
    }
    Ok(true)
}

fn print_summary(summaries: &summary::Summaries) {
    // One `key=value` line per (variant, workload, mutant) group, so
    // the summary is both readable and machine-parseable.
    for ((variant, workload, mutant), s) in summaries {
        let detection_rate = s.detection_rate();
        let median = s
            .median_detection()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string());
        let quartiles = s
            .quartiles()
            .map(|(q25, q75)| format!("{q25}-{q75}"))
            .unwrap_or_else(|| "-".to_string());
        let buckets: Vec<String> = s.detection_buckets.iter().map(|v| v.to_string()).collect();
        println!(
            "variant={variant} workload={workload} mutant={mutant} \
             trials={} found={} not_found={} gave_up={} aborted={} \
             detection_rate={detection_rate:.3} median={median} quartiles={quartiles} \
             buckets=[{}]",
            s.trials,
            s.found,
            s.not_found,
            s.gave_up,
            s.aborted,
            buckets.join(",")
        );
    }
}
