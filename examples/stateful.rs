//! Model-based (stateful) property testing: command selection depends
//! on the current model state, explicit weights shape the operation
//! sequence, and every transition compares the SUT with the model.
//!
//! Run with: `cargo run --example stateful`

use std::cell::Cell;
use std::collections::VecDeque;

const CAPACITY: usize = 4;

/// A bounded FIFO queue that discards its oldest value when a push
/// would exceed capacity.
struct BoundedQueue {
    capacity: usize,
    values: VecDeque<u32>,
}

impl BoundedQueue {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            values: VecDeque::new(),
        }
    }

    fn push(&mut self, value: u32) {
        if self.values.len() == self.capacity {
            self.values.pop_front();
        }
        self.values.push_back(value);
    }

    fn pop(&mut self) -> Option<u32> {
        self.values.pop_front()
    }

    fn snapshot(&self) -> Vec<u32> {
        self.values.iter().copied().collect()
    }
}

#[derive(Debug, Clone, Copy)]
enum Command {
    Push(u32),
    Pop,
}

fn sample_command(ctx: &mut noprop::TestCaseContext, model: &[u32]) -> Command {
    if model.is_empty() {
        return Command::Push(noprop::sample_u32(ctx));
    }

    // A pop is only generated when the current model makes it valid.
    // Pushes are weighted more heavily so full-queue eviction remains
    // common enough to check within a short command sequence.
    match noprop::sample_weighted_index(ctx, &[3, 2]) {
        0 => Command::Push(noprop::sample_u32(ctx)),
        _ => Command::Pop,
    }
}

fn main() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("NOPROP_SEED")?;
    let evictions = Cell::new(0usize);
    let successful_pops = Cell::new(0usize);
    let mut runner = noprop::Runner::new(seed);

    runner.run(256, |ctx| {
        let mut model = Vec::new();
        let mut sut = BoundedQueue::new(CAPACITY);
        let mut history = Vec::new();

        for step in 0..32 {
            let command = sample_command(ctx, &model);
            let causes_eviction = matches!(command, Command::Push(_)) && model.len() == CAPACITY;
            history.push(command);

            let successful_pop = match command {
                Command::Push(value) => {
                    if causes_eviction {
                        model.remove(0);
                    }
                    model.push(value);
                    sut.push(value);
                    false
                }
                Command::Pop => {
                    let expected = if model.is_empty() {
                        None
                    } else {
                        Some(model.remove(0))
                    };
                    let actual = sut.pop();
                    assert_eq!(
                        actual, expected,
                        "step {step}: pop returned different values\n\
                         history: {history:#?}"
                    );
                    expected.is_some()
                }
            };

            // `snapshot` observes state without performing another queue
            // operation, so the check cannot hide a transition bug.
            assert_eq!(
                sut.snapshot(),
                model,
                "step {step}: state mismatch after {command:?}\n\
                 history: {history:#?}"
            );
            if causes_eviction {
                evictions.set(evictions.get() + 1);
            }
            if successful_pop {
                successful_pops.set(successful_pops.get() + 1);
            }
        }
        Ok(())
    })?;

    assert!(
        evictions.get() > 0,
        "no command sequence exercised eviction\n{runner}"
    );
    assert!(
        successful_pops.get() > 0,
        "no command sequence exercised a successful pop\n{runner}"
    );
    println!(
        "stateful property: passed ({} evictions, {} successful pops)",
        evictions.get(),
        successful_pops.get()
    );
    Ok(())
}
