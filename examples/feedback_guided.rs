//! Feedback-guided search (`Runner::run_feedback_guided`) over a real
//! task: a log pipeline ingests lines into a fixed buffer. Long lines
//! (the interesting inputs — they exercise truncation) are reported
//! with `event`, and the search concentrates on them instead of
//! sampling uniformly.
//!
//! Run with: `cargo run --example feedback_guided`

use noprop::TestCaseContext;
use std::cell::Cell;

/// Ingest a log line into a fixed-size buffer, truncating lines that
/// do not fit. Returns the number of bytes written.
fn ingest(line: &str, buf: &mut [u8]) -> usize {
    let n = line.len().min(buf.len());
    buf[..n].copy_from_slice(&line.as_bytes()[..n]);
    n
}

/// A buffer size an operator might configure for one pipeline stage.
const BUF_SIZE: usize = 12;

fn main() -> noprop::TestResult {
    // === event steers the search ===
    //
    // `event("long-line")` marks lines that overflow the buffer — the
    // inputs that exercise the truncation path. Compare the line
    // lengths the two search modes generate: the guided run replays
    // cases that reported the event, so its later half clusters at
    // long lines while the uniform run stays flat.
    fn observe(seed: u64, guided: bool) -> Vec<usize> {
        let observed: Cell<Vec<usize>> = Cell::new(Vec::new());
        let property = |ctx: &mut TestCaseContext| {
            let len = noprop::sample_usize_in(ctx, 0..=24);
            let line = noprop::sample_string(ctx, len);
            let mut buf = [0u8; BUF_SIZE];
            let written = ingest(&line, &mut buf);
            assert!(written <= BUF_SIZE, "ingest must never overflow the buffer");
            if len > BUF_SIZE {
                assert_eq!(written, BUF_SIZE, "overlong lines must be truncated");
            }
            let mut v = observed.take();
            v.push(len);
            observed.set(v);
            if len > BUF_SIZE {
                ctx.event("long-line");
            }
            Ok(())
        };
        let mut runner = noprop::Runner::new(seed);
        if guided {
            runner
                .run_feedback_guided(256, property)
                .expect("feedback-guided run must succeed");
        } else {
            runner.run(256, property).expect("uniform run must succeed");
        }
        observed.into_inner()
    }

    let uniform = observe(7, false);
    let guided = observe(7, true);
    let second_half_mean = |xs: &[usize]| xs[128..].iter().sum::<usize>() as f64 / 128.0;
    let uniform_mean = second_half_mean(&uniform);
    let guided_mean = second_half_mean(&guided);
    println!("uniform run:       second-half mean line length = {uniform_mean:.1}");
    println!("corpus-guided run: second-half mean line length = {guided_mean:.1}");
    println!(
        "the guided run concentrates cases on overlong lines: {} of 256",
        guided.iter().filter(|&&l| l > BUF_SIZE).count()
    );
    println!(
        "the uniform run reaches them by chance: {} of 256",
        uniform.iter().filter(|&&l| l > BUF_SIZE).count()
    );
    assert!(
        guided_mean > uniform_mean,
        "the guided run must steer toward the reported region"
    );

    // === bucket and transition report distributions ===
    //
    // `bucket` reports where a value fell in a caller-designed
    // histogram; `transition` reports an abstract state change. Both
    // widen the feature set the corpus is built from, without
    // steering toward a single region.
    noprop::Runner::new(0xC0FFEE).run_feedback_guided(64, |ctx| {
        let len = noprop::sample_usize_in(ctx, 0..=24);
        let _line = noprop::sample_string(ctx, len);
        ctx.bucket("len-bucket", (len / 4) as u64);
        ctx.transition("ingest", 0, (len % 3) as u64);
        Ok(())
    })?;
    println!("bucket / transition reporting: passed");

    // === coverage gate: require_event ===
    //
    // Reporting an event steers the search, but a run that never
    // reaches the region still passes silently. `Runner::require_event`
    // turns "should reach" into "must reach": when the declared event
    // is not reported even once, the run fails with
    // RequiredEventNotReached instead of passing vacuously.
    let mut runner = noprop::Runner::new(0xFEED);
    runner.require_event("long-line");
    runner.run_feedback_guided(256, |ctx| {
        let len = noprop::sample_usize_in(ctx, 0..=24);
        let line = noprop::sample_string(ctx, len);
        let mut buf = [0u8; BUF_SIZE];
        let _ = ingest(&line, &mut buf);
        if len > BUF_SIZE {
            ctx.event("long-line");
        }
        Ok(())
    })?;
    println!(
        "coverage gate: passed (the `long-line` event was reached {} times)",
        runner.stats().required_event_hits
    );
    Ok(())
}
