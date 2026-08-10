Benchmark
=========

`benchmark` measures how many cases noprop needs to detect known
mutants of small workloads (high-frequency, boundary, combination,
dependent, bst, stepping, stateful) under different generator
variants, and how broad the generated inputs are (semantic buckets;
reported by the dependent workload, whose base variant shows the full
breadth). The guard workload checks that the feedback-guided search
stays bounded (corpus size and feature registry).

```
# Run a single task: print one raw-result JSON line.
cargo run -p benchmark --bin benchmark -- run \
    --workload bst --mutant insert_duplicate_key --variant uniform --seed 0

# Run every task over a seed cohort: one raw-result JSON line per trial.
cargo run -p benchmark --bin benchmark -- run-all \
    --cases 1000 --seeds 0,1,2,3,4,5,6,7 > raw.jsonl

# Regenerate the bucket summary from raw results.
cargo run -p benchmark --bin benchmark -- summary < raw.jsonl
```

The `base` variant (ground-truth SUT) completes every property and is
used to verify the workloads; the comparison variants are `uniform`,
`biased`, `boundary-biased`, and `feedback-guided`. Raw results are
written as format-versioned JSON lines, so summaries can always be
regenerated from a saved cohort. Smoke tests live in
`tests/detection_benchmark.rs`.

These numbers measure only the chosen workloads, mutants, seed
cohort, and case budget. They are not a complete measure of
generator quality: a generator that wins on one target may lose on
another, and detection speed says nothing about shrinking quality.
