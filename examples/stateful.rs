//! Model-based (stateful) property testing: an abstract model and the
//! system under test are driven by the same bounded command loop, and
//! every step checks the SUT against the model.
//!
//! The SUT is an LRU cache — a data structure with subtle eviction
//! rules — and the model is the plain rule it should implement.
//!
//! Run with: `cargo run --example stateful`

use std::collections::{HashMap, VecDeque};

/// An LRU cache standing in for a real system (a session store, a
/// rate-limit table, ...). Inserting beyond capacity evicts the least
/// recently used entry; reading a key refreshes its recency.
///
/// This implementation tracks recency with a per-access clock and
/// finds the eviction victim by scanning — a plausible production
/// choice, and a good place for off-by-one and stale-clock bugs. The
/// model below implements the same rule with a recency-ordered list,
/// so any mismatch between the two is caught within a few commands.
struct LruCache {
    capacity: usize,
    /// key -> (value, last access clock).
    entries: HashMap<u32, (u32, u64)>,
    clock: u64,
}

impl LruCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: HashMap::new(),
            clock: 0,
        }
    }

    fn get(&mut self, key: u32) -> Option<u32> {
        let entry = self.entries.get_mut(&key)?;
        self.clock += 1;
        entry.1 = self.clock;
        Some(entry.0)
    }

    fn put(&mut self, key: u32, value: u32) {
        if let Some(entry) = self.entries.get_mut(&key) {
            *entry = (value, self.clock);
            return;
        }
        if self.entries.len() >= self.capacity
            && let Some(victim) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, last_used))| *last_used)
                .map(|(&k, _)| k)
        {
            self.entries.remove(&victim);
        }
        self.entries.insert(key, (value, self.clock));
    }
}

/// The abstract model of an LRU cache: a recency-ordered list of keys
/// and a key -> value map, with the same eviction rule.
struct Model {
    capacity: usize,
    order: VecDeque<u32>,
    values: HashMap<u32, u32>,
}

impl Model {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            order: VecDeque::new(),
            values: HashMap::new(),
        }
    }

    fn get(&mut self, key: u32) -> Option<u32> {
        let value = self.values.get(&key).copied()?;
        self.touch(key);
        Some(value)
    }

    fn put(&mut self, key: u32, value: u32) {
        if let Some(entry) = self.values.get_mut(&key) {
            *entry = value;
            self.touch(key);
            return;
        }
        if self.values.len() >= self.capacity
            && let Some(evicted) = self.order.pop_back()
        {
            self.values.remove(&evicted);
        }
        self.values.insert(key, value);
        self.order.push_front(key);
    }

    fn touch(&mut self, key: u32) {
        if let Some(pos) = self.order.iter().position(|&k| k == key) {
            self.order.remove(pos);
        }
        self.order.push_front(key);
    }
}

/// Pick the next command: reads and writes of small integer keys, so
/// cache hits, misses, and evictions all occur within a run.
fn sample_command(ctx: &mut noprop::TestCaseContext) -> (u32, Option<u32>) {
    let key = noprop::sample_usize_in(ctx, 0..8) as u32;
    let write = noprop::sample_bool(ctx);
    let value = write.then(|| noprop::sample_u32(ctx));
    (key, value)
}

fn main() -> noprop::TestResult {
    // `transition` reports each model step's cache-size change to the
    // feedback-guided search, so `run_feedback_guided` steers toward
    // command chains that exercise different fullness transitions
    // (grow / stay / evict) instead of restarting the cache from
    // scratch every case.
    let seed = noprop::seed_from_env_or_time("NOPROP_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run_feedback_guided(256, |ctx| {
        let mut model = Model::new(4);
        let mut sut = LruCache::new(4);
        let mut prev_size = 0usize;

        for step in 0..64 {
            let (key, write) = sample_command(ctx);
            match write {
                None => {
                    let expected = model.get(key);
                    let actual = sut.get(key);
                    assert_eq!(
                        expected, actual,
                        "step {step}: get({key}) — model said {expected:?}, SUT returned {actual:?}"
                    );
                }
                Some(value) => {
                    model.put(key, value);
                    sut.put(key, value);
                    assert_eq!(
                        sut.get(key),
                        Some(value),
                        "step {step}: put({key}, {value}) must be readable back"
                    );
                }
            }
            // (prev_size, size) both live in 0..=capacity (4), so the
            // whole run reports at most (capacity + 1)^2 = 25 distinct
            // features — a bounded (from, to) pair rather than the
            // per-step counter that would grow the feature registry
            // without bound.
            let size = model.order.len();
            ctx.transition("lru-size", prev_size as u64, size as u64);
            prev_size = size;
        }
        Ok(())
    })?;
    println!("stateful property: passed (256 cases of 64-step get/put chains)");
    Ok(())
}
