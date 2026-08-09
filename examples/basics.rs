//! Basic property-based testing with noprop: the minimal shape, the
//! two pitfalls that trip up first-time users, and the short idioms
//! for random-length collections and boundary values.
//!
//! Run with: `cargo run --example basics`

/// A property is a closure `Fn(&mut TestCaseContext) -> TestResult`:
/// plain Rust control flow, no combinator DSL. The runner invokes it
/// up to `cases` times with a fresh draw of random values each time.
fn main() -> noprop::TestResult {
    // === The minimal shape ===
    //
    // The runner seeds a deterministic random stream, so a failing
    // case can always be reproduced from the seed (see `reproduce`).
    noprop::Runner::new(0xDEAD_BEEF).run(1024, |ctx| {
        let a = noprop::sample_u32(ctx);
        let b = noprop::sample_u32(ctx);
        assert_eq!(a.wrapping_add(b), b.wrapping_add(a));
        Ok(())
    })?;
    println!("minimal property: passed");

    // === Pitfall 1: the closure is `Fn`, not `FnMut` ===
    //
    // The property closure cannot capture state by mutable reference.
    // Reach for interior mutability when a case needs to observe
    // something across invocations (a step counter, an "already seen"
    // flag), and the intent stays spelled out in the code.
    let low_draws = std::cell::Cell::new(0usize);
    noprop::Runner::new(0).run(64, |ctx| {
        let x = noprop::sample_u8(ctx);
        if x < 16 {
            low_draws.set(low_draws.get() + 1);
        }
        Ok(())
    })?;
    println!(
        "interior mutability: passed ({} low draws observed)",
        low_draws.get()
    );

    // === Pitfall 2: fix the seed to reproduce failures ===
    //
    // A fresh seed every run means a failure cannot be replayed. Fix
    // the seed (or read it from an environment variable) so a failing
    // seed from a report can be re-run verbatim. The same seed always
    // produces the same failure case index.
    let seed = 0xBAD_CAFE_1234_5678;
    let run = || {
        let case = std::cell::Cell::new(0usize);
        noprop::Runner::new(seed).run(64, |ctx| {
            let n = case.get();
            case.set(n + 1);
            let _ = noprop::sample_u32(ctx);
            if n >= 3 {
                panic!("boom at case {n}");
            }
            Ok(())
        })
    };
    // The failure is deliberate; silence the default panic hook so
    // the comparison below is the only output.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let a = run().expect_err("this seed must fail");
    let b = run().expect_err("the same seed must fail identically");
    std::panic::set_hook(default_hook);
    assert_eq!(a.case_index(), b.case_index());
    println!(
        "seed reproducibility: passed (failure at case {})",
        a.case_index()
    );

    // === Idiom 1: random-length collections ===
    //
    // The length-taking primitives (`sample_bytes_vec`,
    // `sample_string`, ...) take an exact length. To pick the length
    // randomly, draw it first with `sample_usize_in` — never with
    // `%`, which is biased and overflows at `usize::MAX`.
    noprop::Runner::new(0xC0FFEE).run(64, |ctx| {
        let max_len = 32;
        let len = noprop::sample_usize_in(ctx, 0..=max_len);
        let bytes = noprop::sample_bytes_vec(ctx, len);
        let s = noprop::sample_string(ctx, len);
        assert_eq!(bytes.len(), len);
        assert_eq!(s.chars().count(), len);
        Ok(())
    })?;
    println!("random-length collections: passed");

    // === Idiom 2: boundary values ===
    //
    // A uniform draw hits domain boundaries (0, `u32::MAX`, an MTU
    // size, ...) with vanishing probability. `sample_with_boundaries`
    // mixes a few candidates in with an exact probability.
    let low = std::cell::Cell::new(0usize);
    let boundary = std::cell::Cell::new(0usize);
    noprop::Runner::new(0xFEED).run(256, |ctx| {
        let x = noprop::sample_with_boundaries(
            ctx,
            &[0, 1500, u32::MAX],
            noprop::Ratio::ONE_TENTH,
            noprop::sample_u32,
        );
        if x == 0 || x == 1500 || x == u32::MAX {
            boundary.set(boundary.get() + 1);
        } else {
            low.set(low.get() + 1);
        }
        Ok(())
    })?;
    println!(
        "boundary values: passed ({} boundary draws out of {})",
        boundary.get(),
        low.get() + boundary.get()
    );

    Ok(())
}
