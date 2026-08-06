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

    // Subcommand dispatch: each handler early-returns `Ok(true)` on
    // success (or help mode), so `finish()` below always runs and
    // reports unconsumed arguments and prints the help in help mode.
    let _ = run_cmd(&mut args)? || run_all_cmd(&mut args)? || summary_cmd(&mut args)?;

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
        .example("bst")
        .take(args)
        .then(|o| o.value().parse())?;
    let mutant_name: String = opt("mutant")
        .ty("NAME")
        .doc("Mutant name")
        .example("insert_duplicate_key")
        .take(args)
        .then(|o| o.value().parse())?;
    let variant_name: String = opt("variant")
        .ty("NAME")
        .doc("Search variant: uniform | biased | targeted | semantic-only | semantic-with-priority (or base for the ground-truth SUT)")
        .example("uniform")
        .take(args)
        .then(|o| o.value().parse())?;
    let seed: u64 = opt("seed")
        .ty("N")
        .doc("Seed for the run")
        .example("0")
        .take(args)
        .then(|o| o.value().parse())?;
    let iterations: usize = opt("iterations")
        .ty("N")
        .doc("Accepted iteration budget")
        .default("1000")
        .take(args)
        .then(|o| o.value().parse())?;
    if iterations == 0 {
        return Err(Error::other(
            args,
            "iterations must be at least 1 (a zero budget produces a vacuous result)",
        ));
    }

    if args.metadata().help_mode {
        return Ok(true);
    }

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
    let variant = variants::Variant::from_str(&variant_name).ok_or_else(|| {
        Error::other(
            args,
            format!(
                "unknown variant {variant_name:?}; available: base, uniform, biased, \
                 targeted, semantic-only, semantic-with-priority"
            ),
        )
    })?;

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
    if iterations == 0 {
        return Err(Error::other(
            args,
            "iterations must be at least 1 (a zero budget produces a vacuous result)",
        ));
    }
    let seeds_text: String = opt("seeds")
        .ty("LIST")
        .doc("Comma-separated seed list, e.g. 0,1,2,3")
        .example("0,1,2,3")
        .take(args)
        .then(|o| o.value().parse())?;

    if args.metadata().help_mode {
        return Ok(true);
    }

    let mut seeds: Vec<u64> = seeds_text
        .split(',')
        .map(|s| s.trim().parse::<u64>())
        .collect::<Result<_, _>>()
        .map_err(|e| Error::other(args, format!("invalid seed list {seeds_text:?}: {e}")))?;
    seeds.sort_unstable();
    seeds.dedup();

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

    if args.metadata().help_mode {
        return Ok(true);
    }

    let stdin = std::io::stdin();
    let (summaries, skipped) = summary::read_summaries(stdin.lock());
    print_summary(&summaries);
    if skipped > 0 {
        // Partial results stay on stdout; the non-zero exit flags the
        // corrupted input to pipelines.
        return Err(Error::other(
            args,
            format!("skipped {skipped} malformed raw-result line(s)"),
        ));
    }
    Ok(true)
}

fn print_summary(summaries: &summary::Summaries) {
    // One `key=value` line per (variant, workload, mutant) group, so
    // the summary is both readable and machine-parseable. The bucket
    // bounds are printed once so group lines stay self-explanatory.
    let bounds: Vec<String> = summary::DETECTION_BUCKETS
        .iter()
        .scan(1, |prev, bound| {
            let label = format!("{prev}-{}", bound - 1);
            *prev = *bound;
            Some(label)
        })
        .chain(std::iter::once(format!(
            "{}+",
            summary::DETECTION_BUCKETS.last().expect("non-empty bounds")
        )))
        .collect();
    println!("bucket_bounds={}", bounds.join(","));
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
        let candidate_median = s
            .median_candidates()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string());
        let candidate_quartiles = s
            .candidate_quartiles()
            .map(|(q25, q75)| format!("{q25}-{q75}"))
            .unwrap_or_else(|| "-".to_string());
        let candidate_buckets: Vec<String> =
            s.candidate_buckets.iter().map(|v| v.to_string()).collect();
        let discovered = s
            .median_discovered_features()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string());
        let corpus = s
            .median_max_corpus_size()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string());
        println!(
            "variant={variant} workload={workload} mutant={mutant} \
             trials={} found={} not_found={} gave_up={} aborted={} \
             detection_rate={detection_rate:.3} median={median} quartiles={quartiles} \
             buckets=[{}] candidate_median={candidate_median} \
             candidate_quartiles={candidate_quartiles} candidate_buckets=[{}] \
             discovered_features_median={discovered} max_corpus_size_median={corpus}",
            s.trials,
            s.found,
            s.not_found,
            s.gave_up,
            s.aborted,
            buckets.join(","),
            candidate_buckets.join(",")
        );
    }
}
