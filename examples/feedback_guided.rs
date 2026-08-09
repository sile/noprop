//! Feedback-guided search (`Runner::run_feedback_guided`): reporting
//! semantic feedback (`event`, `bucket`, `transition`) lets the search
//! concentrate cases on the interesting input region instead of
//! sampling uniformly.
//!
//! Run with: `cargo run --example feedback_guided`

use noprop::TestCaseContext;
use std::cell::Cell;

fn main() -> noprop::TestResult {
    // === event steers the search ===
    //
    // `event("high")` marks the input region the search should
    // concentrate on. Compare the input distributions of the two
    // search modes: the guided run replays cases that reported the
    // event, so its later half clusters above 900 while the uniform
    // run stays flat.
    fn observe(seed: u64, guided: bool) -> Vec<usize> {
        let observed: Cell<Vec<usize>> = Cell::new(Vec::new());
        let property = |ctx: &mut TestCaseContext| {
            let x = noprop::sample_usize_in(ctx, 0..1000);
            let mut v = observed.take();
            v.push(x);
            observed.set(v);
            if x > 900 {
                ctx.event("high");
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
    let second_half_median = |xs: &mut [usize]| {
        xs.sort_unstable();
        xs[128..][xs.len() / 2 - 128]
    };
    let mut uniform_sorted = uniform.clone();
    let mut guided_sorted = guided.clone();
    let uniform_median = second_half_median(&mut uniform_sorted);
    let guided_median = second_half_median(&mut guided_sorted);
    println!("uniform run:       second-half median = {uniform_median}");
    println!("corpus-guided run: second-half median = {guided_median}");
    println!(
        "the guided run concentrates cases on the `high` region (>900): {} of 256",
        guided.iter().filter(|&&x| x > 900).count()
    );
    println!(
        "the uniform run reaches it by chance: {} of 256",
        uniform.iter().filter(|&&x| x > 900).count()
    );
    assert!(
        guided_median > uniform_median,
        "the guided run must steer toward the reported region"
    );

    // === bucket and transition report distributions ===
    //
    // `bucket` reports where a value fell in a caller-designed
    // histogram; `transition` reports an abstract state change. Both
    // widen the feature set the corpus is built from, without
    // steering toward a single region.
    noprop::Runner::new(0xC0FFEE).run_feedback_guided(64, |ctx| {
        let x = noprop::sample_usize_in(ctx, 0..1000);
        ctx.bucket("range", (x / 100) as u64);
        ctx.transition("step", 0, (x % 10) as u64);
        Ok(())
    })?;
    println!("bucket / transition reporting: passed");
    Ok(())
}
