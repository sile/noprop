//! Curated-mutation target: a small binary search tree compared
//! against the standard library map. The mutant forgets to skip
//! duplicate keys on insert, so a key inserted twice duplicates the
//! key in the in-order traversal (the order stays sorted; the
//! multiplicity breaks the equality with the model).
//!
//! Uniform generation over a wide key range draws a duplicate only
//! after many cases, so the mutant is detected late (or not at
//! all with a small budget); the biased variant narrows the key range
//! so duplicates appear within the first cases.

use super::{Observe, Task, Workload};

/// Minimal binary search tree used as the SUT.
#[derive(Default)]
struct Bst {
    root: Option<Box<Node>>,
}

struct Node {
    key: u32,
    left: Option<Box<Node>>,
    right: Option<Box<Node>>,
}

impl Bst {
    /// Insert `key`. Under the mutant, an already-present key is
    /// inserted again (duplicating it in the in-order traversal).
    fn insert(&mut self, key: u32, duplicate_bug: bool) {
        if !duplicate_bug && self.contains(key) {
            return;
        }
        self.root = insert_into(self.root.take(), key);
    }

    fn contains(&self, key: u32) -> bool {
        let mut node = self.root.as_deref();
        while let Some(n) = node {
            if key == n.key {
                return true;
            }
            node = if key < n.key {
                n.left.as_deref()
            } else {
                n.right.as_deref()
            };
        }
        false
    }

    fn delete(&mut self, key: u32) {
        self.root = delete_from(self.root.take(), key);
    }

    /// In-order traversal: sorted keys iff the tree is a valid BST.
    fn keys(&self) -> Vec<u32> {
        let mut out = Vec::new();
        collect(&self.root, &mut out);
        out
    }
}

fn insert_into(node: Option<Box<Node>>, key: u32) -> Option<Box<Node>> {
    let Some(mut node) = node else {
        return Some(Box::new(Node {
            key,
            left: None,
            right: None,
        }));
    };
    if key < node.key {
        node.left = insert_into(node.left.take(), key);
    } else {
        node.right = insert_into(node.right.take(), key);
    }
    Some(node)
}

fn delete_from(node: Option<Box<Node>>, key: u32) -> Option<Box<Node>> {
    let mut node = node?;
    if key < node.key {
        node.left = delete_from(node.left.take(), key);
        return Some(node);
    }
    if key > node.key {
        node.right = delete_from(node.right.take(), key);
        return Some(node);
    }
    // Key found: replace with the in-order successor (or the other
    // child when one side is empty).
    match (node.left.take(), node.right.take()) {
        (None, right) => right,
        (left, None) => left,
        (left, Some(right)) => {
            let successor_key = successor_key(&right);
            let right = delete_from(Some(right), successor_key);
            Some(Box::new(Node {
                key: successor_key,
                left,
                right,
            }))
        }
    }
}

fn successor_key(node: &Node) -> u32 {
    let mut node = node;
    while let Some(next) = node.left.as_deref() {
        node = next;
    }
    node.key
}

fn collect(node: &Option<Box<Node>>, out: &mut Vec<u32>) {
    if let Some(node) = node {
        collect(&node.left, out);
        out.push(node.key);
        collect(&node.right, out);
    }
}

/// Generate an operation sequence: up to `max_ops` insert/delete
/// operations on keys in `0..1024` (biased: mostly `0..8`).
fn run(
    sut_mutant: bool,
    biased: bool,
    ctx: &mut noprop::TestCaseContext,
    _obs: &Observe,
) -> Result<(), String> {
    let mut bst = Bst::default();
    let mut model = std::collections::BTreeMap::new();

    let max_ops = noprop::sample_usize_in(ctx, 0..16);
    for _ in 0..max_ops {
        let key = if biased {
            // 50% a small key (0..8), otherwise uniform 0..1024, so
            // duplicates (which trip the mutant) are common.
            if noprop::sample_bool(ctx) {
                noprop::sample_usize_in(ctx, 0..8) as u32
            } else {
                noprop::sample_usize_in(ctx, 0..1024) as u32
            }
        } else {
            noprop::sample_usize_in(ctx, 0..1024) as u32
        };
        // 80% insert under the biased variant, 50% otherwise.
        let insert = if biased {
            noprop::sample_usize_in(ctx, 0..10) < 8
        } else {
            noprop::sample_bool(ctx)
        };
        if insert {
            bst.insert(key, sut_mutant);
            model.insert(key, ());
        } else {
            bst.delete(key);
            model.remove(&key);
        }
    }

    // Property: the BST's in-order keys must match the model exactly.
    let actual = bst.keys();
    let expected: Vec<u32> = model.keys().copied().collect();
    if actual != expected {
        return Err(format!("bst keys {actual:?} != model keys {expected:?}"));
    }
    Ok(())
}

pub(crate) const WORKLOAD: Workload = Workload {
    name: "bst",
    description: "binary search tree vs standard-library map; duplicate insert not skipped",
    tasks: &[Task {
        mutant: "insert_duplicate_key",
        base: run_base,
        uniform: run_uniform,
        biased: run_biased,
        // The generator draws only bounded ranges and booleans, which
        // the generic boundary mix does not wrap, so the bb property
        // is the uniform one.
        bb: run_uniform,
    }],
};

fn run_base(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(false, false, ctx, obs)
}
fn run_uniform(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(true, false, ctx, obs)
}
fn run_biased(ctx: &mut noprop::TestCaseContext, obs: &Observe) -> Result<(), String> {
    run(true, true, ctx, obs)
}
