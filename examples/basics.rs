//! The minimal noprop test shape: choose a reproducible seed, sample
//! inputs, call ordinary Rust code, and assert a property.
//!
//! Run with: `cargo run --example basics`

fn encode_pair(left: u16, right: u16) -> [u8; 4] {
    let mut encoded = [0; 4];
    encoded[..2].copy_from_slice(&left.to_be_bytes());
    encoded[2..].copy_from_slice(&right.to_be_bytes());
    encoded
}

fn decode_pair(encoded: [u8; 4]) -> (u16, u16) {
    let left = u16::from_be_bytes([encoded[0], encoded[1]]);
    let right = u16::from_be_bytes([encoded[2], encoded[3]]);
    (left, right)
}

fn main() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("NOPROP_SEED")?;

    noprop::Runner::new(seed).run(256, |ctx| {
        let pair = (noprop::sample_u16(ctx), noprop::sample_u16(ctx));
        assert_eq!(decode_pair(encode_pair(pair.0, pair.1)), pair);
        Ok(())
    })?;

    println!("pair round-trip property: passed");
    Ok(())
}
