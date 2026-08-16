//! Explicit search-space design in one property: a weighted format
//! branch, a dependent payload length, domain boundaries with an exact
//! probability, and coverage gates checked after the run.
//!
//! Run with: `cargo run --example search_space`

use std::cell::Cell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Version {
    V1,
    V2,
}

impl Version {
    fn tag(self) -> u8 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
        }
    }

    fn max_payload_len(self) -> usize {
        match self {
            Self::V1 => 8,
            Self::V2 => 64,
        }
    }
}

fn encode_packet(version: Version, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= version.max_payload_len());
    let mut encoded = Vec::new();
    encoded.push(version.tag());
    encoded.push(payload.len() as u8);
    encoded.extend_from_slice(payload);
    encoded
}

fn decode_packet(encoded: &[u8]) -> Option<(Version, Vec<u8>)> {
    let (&tag, rest) = encoded.split_first()?;
    let (&declared_len, payload) = rest.split_first()?;
    let version = match tag {
        1 => Version::V1,
        2 => Version::V2,
        _ => return None,
    };
    if payload.len() != usize::from(declared_len) || payload.len() > version.max_payload_len() {
        return None;
    }
    Some((version, payload.to_vec()))
}

fn main() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("NOPROP_SEED")?;
    let empty_payloads = Cell::new(0usize);
    let maximum_v2_payloads = Cell::new(0usize);
    let mut runner = noprop::Runner::new(seed);

    runner.run(512, |ctx| {
        // Keep the common V1 format three times as likely as V2. The
        // weights are visible next to the branch they control.
        let version = match noprop::sample_weighted_index(ctx, &[3, 1]) {
            0 => Version::V1,
            _ => Version::V2,
        };

        // The legal payload length depends on the version drawn above.
        // Half of the draws deliberately select an empty, singleton, or
        // maximum-length payload; the rest cover the interior uniformly.
        let max_len = version.max_payload_len();
        let boundaries = [0, 1, max_len];
        let len =
            noprop::sample_with_boundaries(ctx, &boundaries, noprop::Ratio::one_nth(2), |ctx| {
                noprop::sample_usize_in(ctx, 0..=max_len)
            });
        let payload = noprop::sample_bytes_vec(ctx, len);

        assert_eq!(
            decode_packet(&encode_packet(version, &payload)),
            Some((version, payload))
        );

        // Count coverage only after the property has passed for the
        // relevant class. A failing case must not satisfy the gate.
        if len == 0 {
            empty_payloads.set(empty_payloads.get() + 1);
        }
        if version == Version::V2 && len == Version::V2.max_payload_len() {
            maximum_v2_payloads.set(maximum_v2_payloads.get() + 1);
        }
        Ok(())
    })?;

    assert!(
        empty_payloads.get() > 0,
        "no case exercised an empty payload\n{runner}"
    );
    assert!(
        maximum_v2_payloads.get() > 0,
        "no case exercised a maximum-length V2 payload\n{runner}"
    );
    println!(
        "search-space property: passed ({} empty, {} maximum-length V2)",
        empty_payloads.get(),
        maximum_v2_payloads.get()
    );
    Ok(())
}
