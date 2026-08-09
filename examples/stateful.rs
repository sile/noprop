//! Model-based (stateful) property testing: an abstract model and the
//! system under test are driven by the same bounded command loop, and
//! every step checks the SUT against the model.
//!
//! Run with: `cargo run --example stateful`

use noprop::TestCaseContext;

/// The commands the model and the SUT both understand.
enum Command {
    Push(u32),
    Pop,
}

/// A stack with a fixed capacity, standing in for a real system.
///
/// The model below mirrors the capacity decision, so a bug in the SUT
/// (e.g. forgetting to decrement the length) fails one of the equality
/// checks within a few commands.
struct Stack {
    buf: Vec<u32>,
    len: usize,
}

impl Stack {
    const CAPACITY: usize = 16;

    fn new() -> Self {
        Self {
            buf: vec![0; Self::CAPACITY],
            len: 0,
        }
    }

    /// Push `v`, returning `false` when the stack is full. The caller
    /// mirrors the result in the model so both stay in lockstep.
    fn push(&mut self, v: u32) -> bool {
        if self.len >= Self::CAPACITY {
            return false;
        }
        self.buf[self.len] = v;
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<u32> {
        if self.len == 0 {
            None
        } else {
            self.len -= 1;
            Some(self.buf[self.len])
        }
    }
}

/// Pick the next command: pushes are more common than pops so the
/// stack actually grows and shrinks over a run.
fn sample_command(ctx: &mut TestCaseContext) -> Command {
    match noprop::sample_usize_in(ctx, 0..8) {
        0 => Command::Pop,
        _ => Command::Push(noprop::sample_u32(ctx)),
    }
}

fn main() -> noprop::TestResult {
    // `transition` reports each model step to the corpus-guided
    // search, so `run_feedback_guided` steers toward longer command
    // chains instead of restarting the model from scratch every case.
    let mut runner = noprop::Runner::new(0xDEAD_BEEF);
    runner.run_feedback_guided(256, |ctx| {
        let mut model: Vec<u32> = Vec::new();
        let mut sut = Stack::new();

        for step in 0..64 {
            let cmd = sample_command(ctx);
            let (model_len, model_top) = match cmd {
                Command::Push(v) => {
                    if sut.push(v) {
                        model.push(v);
                    }
                    (model.len(), None)
                }
                Command::Pop => {
                    let expected = model.pop();
                    let actual = sut.pop();
                    assert_eq!(
                        expected, actual,
                        "step {step}: model said {expected:?}, SUT returned {actual:?}"
                    );
                    (model.len(), actual)
                }
            };
            ctx.transition("stack", step as u64, model_len as u64);
            let _ = model_top;
        }
        Ok(())
    })?;
    println!("stateful property: passed (256 cases of 64-step command chains)");
    Ok(())
}
