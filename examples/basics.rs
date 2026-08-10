//! Basic property-based testing with noprop: the minimal shape
//! against a real function, the two pitfalls that trip up first-time
//! users, and the short idioms for random-length collections and
//! boundary values.
//!
//! Run with: `cargo run --example basics`

/// Format a config line `name=port` — the kind of small utility a real
/// program uses to serialize one record of its configuration. The port
/// must be non-zero.
fn format_config(name: &str, port: u16) -> Result<String, String> {
    if name.is_empty() {
        return Err("name must not be empty".into());
    }
    if port == 0 {
        return Err("port must be non-zero".into());
    }
    Ok(format!("{name}={port}"))
}

fn main() -> noprop::TestResult {
    // The seed comes from the environment: `NOPROP_SEED` overrides,
    // unset falls back to the clock. A failing seed printed in a
    // report can be re-run verbatim by setting it — see the
    // `reproduce` example for the full workflow.
    let seed = noprop::seed_from_env_or_time("NOPROP_SEED")?;

    // === The minimal shape ===
    //
    // A property is a closure `Fn(&mut TestCaseContext) -> TestResult`:
    // plain Rust control flow, no combinator DSL. The common pattern
    // is to draw the inputs with the sampling primitives and feed
    // them to the function under test.
    noprop::Runner::new(seed).run(256, |ctx| {
        let name = noprop::sample_string(ctx, 16);
        let port = noprop::sample_u16(ctx);
        match format_config(&name, port) {
            Ok(line) => {
                assert_eq!(line, format!("{name}={port}"));
            }
            Err(msg) => {
                assert!(name.is_empty() || port == 0, "unexpected error: {msg}");
            }
        }
        Ok(())
    })?;
    println!("format_config property: passed");

    // === Pitfall 1: the closure is `Fn`, not `FnMut` ===
    //
    // The property closure cannot capture state by mutable reference.
    // Most properties need no shared state at all — everything lives
    // in local variables inside the closure. Reach for interior
    // mutability only for the rare cross-case observation (a debug
    // counter, a report sink), and spell the intent out with `Cell`.
    let long_names = std::cell::Cell::new(0usize);
    noprop::Runner::new(seed).run(64, |ctx| {
        let name = noprop::sample_string(ctx, 16);
        if name.chars().count() > 12 {
            long_names.set(long_names.get() + 1);
        }
        Ok(())
    })?;
    println!(
        "interior mutability: passed ({} long names observed)",
        long_names.get()
    );

    // === Pitfall 2: the same seed reproduces the same failure ===
    //
    // Without a fixed seed a failure cannot be replayed. The failure
    // here is value-dependent — roughly half of the draws violate the
    // assertion — so which case index fails depends on the seed. The
    // same seed must always produce the same case index. A fixed seed
    // (not the env-derived `seed` above) is required so the two runs
    // below compare like-for-like; use its own binding so the outer
    // `seed` keeps its env-driven value for the idioms that follow.
    let pitfall_seed = 0x00FF_00FF_00FF_00FF_u64;
    let run = || {
        noprop::Runner::new(pitfall_seed).run(64, |ctx| {
            let x = noprop::sample_u32(ctx);
            assert!(x < 0x8000_0000, "high bit set: {x:#010x}");
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
    noprop::Runner::new(seed).run(64, |ctx| {
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
    // A uniform draw hits domain boundaries (0, `u16::MAX`, ...) with
    // vanishing probability. `sample_with_boundaries` mixes a few
    // candidates in with an exact probability, so each boundary gets
    // exercised — here each one takes a different code path.
    noprop::Runner::new(seed).run(256, |ctx| {
        let port = noprop::sample_with_boundaries(
            ctx,
            &[0, 1500, u16::MAX],
            noprop::Ratio::one_nth(10),
            noprop::sample_u16,
        );
        // The three boundary candidates each map to a distinct
        // outcome of format_config.
        match port {
            0 => {
                assert_eq!(
                    format_config("svc", port),
                    Err("port must be non-zero".into())
                );
            }
            1500 | u16::MAX => {
                assert_eq!(format_config("svc", port), Ok(format!("svc={port}")));
            }
            _ => {}
        }
        Ok(())
    })?;
    println!("boundary values: passed");

    Ok(())
}
