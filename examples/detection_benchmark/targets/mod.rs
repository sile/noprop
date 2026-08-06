//! Detection-benchmark workloads: a workload is a pair of SUT
//! implementations (base and named mutants) plus the generator and
//! property that exercise them.

mod boundary;
mod bst;
mod combination;
mod dependent;
mod guard;
mod high_frequency;
mod stateful;
mod stepping;

pub(crate) use self::boundary::WORKLOAD as BOUNDARY;
pub(crate) use self::bst::WORKLOAD as BST;
pub(crate) use self::combination::WORKLOAD as COMBINATION;
pub(crate) use self::dependent::WORKLOAD as DEPENDENT;
pub(crate) use self::guard::WORKLOAD as GUARD;
pub(crate) use self::high_frequency::WORKLOAD as HIGH_FREQUENCY;
pub(crate) use self::stateful::WORKLOAD as STATEFUL;
pub(crate) use self::stepping::WORKLOAD as STEPPING;

/// A property run: generate an input from `ctx`, exercise the SUT, and
/// return `Err` when the property fails.
///
/// `observe` collects workload-specific measurements (e.g. semantic
/// buckets reached by the dependent workload); workloads that do not
/// measure anything ignore it. It is passed as an argument instead of a
/// captured cell so properties stay plain function pointers.
pub(crate) type Property = fn(&mut TestCaseContext, &Observe) -> Result<(), String>;

/// One named mutant of a workload: the base SUT must pass the property
/// for any generated input, while the mutant SUT fails for known
/// witnesses.
pub(crate) struct Task {
    pub mutant: &'static str,
    /// Base SUT under uniform generation. Ground-truth check: must
    /// complete the property for any input.
    pub base: Property,
    /// Mutant SUT under uniform generation.
    pub uniform: Property,
    /// Mutant SUT under explicitly biased generation.
    pub biased: Property,
}

pub(crate) struct Workload {
    pub name: &'static str,
    pub description: &'static str,
    pub tasks: &'static [Task],
}

/// All registered workloads, keyed by `Workload::name`.
pub(crate) const WORKLOADS: &[Workload] = &[
    HIGH_FREQUENCY,
    BOUNDARY,
    COMBINATION,
    DEPENDENT,
    BST,
    STEPPING,
    STATEFUL,
    GUARD,
];

/// Observation sink for workload-specific measurements.
///
/// The harness owns one sink per task run and reads the collected
/// (label, count) pairs after the run to place them in the raw result.
/// Counts are aggregated per label (sorted for determinism), so the
/// raw result stays small regardless of how many cases a run has.
#[derive(Default)]
pub(crate) struct Observe {
    counts: std::cell::RefCell<std::collections::BTreeMap<&'static str, u64>>,
}

impl Observe {
    pub fn add(&self, label: &'static str, amount: u64) {
        *self.counts.borrow_mut().entry(label).or_insert(0) += amount;
    }

    pub fn take(&self) -> Vec<(&'static str, u64)> {
        std::mem::take(&mut *self.counts.borrow_mut())
            .into_iter()
            .collect()
    }
}

use noprop::TestCaseContext;

/// Look up a workload by name.
pub(crate) fn workload(name: &str) -> Option<&'static Workload> {
    WORKLOADS.iter().find(|w| w.name == name)
}

/// Look up a task (mutant) within a workload by name.
pub(crate) fn task<'a>(workload: &'a Workload, mutant: &str) -> Option<&'a Task> {
    workload.tasks.iter().find(|t| t.mutant == mutant)
}
