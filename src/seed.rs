//! Environment-variable helper for obtaining the seed
//! [`Runner::new`](crate::Runner::new) takes.
//!
//! The helper is opt-in and is read only when the caller invokes it,
//! so the "no implicit I/O" contract of the rest of the crate is
//! preserved — `TestCaseContext::new` and `Runner::run` never touch
//! the environment or the clock on their own.
//!
//! The [`Runner::new`](crate::Runner::new) rustdoc shows the intended
//! calling shape.

use std::env;
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::TestResult;

/// Parse `raw` as a seed value.
///
/// Accepts a plain decimal `u64` or a `0x`-prefixed hex value, with
/// optional `_` separators (e.g. `1_000_000`, `0xDEAD_BEEF`). Hex is
/// the format failure reports print (`{:#018x}`), so a seed pasted
/// from a failure report parses back verbatim.
fn parse_seed(var: &str, raw: &str) -> TestResult<u64> {
    let trimmed = raw.trim();
    let (radix, digits) = match trimmed.strip_prefix("0x") {
        Some(rest) => (16, rest),
        None => (10, trimmed),
    };
    let cleaned: String = digits.chars().filter(|c| *c != '_').collect();
    u64::from_str_radix(&cleaned, radix).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "environment variable {var:?} has an invalid value {raw:?}: {err}; \
                 expected a decimal integer or a 0x-prefixed hex value (e.g. 1_000_000, 0xDEAD_BEEF)"
            ),
        )
        .into()
    })
}

/// Read `var` as a `u64` seed, or derive one from the current time when
/// it is not set.
///
/// Behavior:
///
/// - `var` unset (`NotPresent`) — returns a seed derived from
///   `SystemTime::now() - UNIX_EPOCH` in nanoseconds, cast to `u64`.
///   If the system clock is before the Unix epoch the fallback is `0`
///   (still a legitimate seed for the internal PRNG).
/// - `var` set to a valid `u64` (decimal or `0x`-prefixed hex, with
///   optional `_` separators) — returns that value.
/// - `var` set to a value that fails to parse — returns a boxed
///   [`io::Error`] naming the variable, the raw value, and the parse
///   error, with the accepted prefixes illustrated.
/// - `var` set to a non-Unicode value — returns a boxed [`io::Error`]
///   naming the variable.
///
/// # Examples
///
/// ```
/// let seed = noprop::seed_from_env_or_time("MYAPP_SEED")
///     .expect("MYAPP_SEED, if set, must parse as u64");
/// let _ = noprop::Runner::new(seed);
/// ```
pub fn seed_from_env_or_time(var: &str) -> TestResult<u64> {
    match env::var(var) {
        Ok(raw) => parse_seed(var, &raw),
        Err(env::VarError::NotPresent) => Ok(time_seed()),
        Err(env::VarError::NotUnicode(_)) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("environment variable {var:?} is not valid UTF-8"),
        )
        .into()),
    }
}

/// Best-effort time-based seed. Wraps to `0` if the system clock is
/// before `UNIX_EPOCH` (rare but not UB).
fn time_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // env::set_var is `unsafe` in Rust 2024 and the crate forbids
    // unsafe_code, so the tests exercise the pieces that don't touch
    // process-wide state: parse_seed for the accepted forms and
    // error kinds, and time_seed for the fallback path. The public
    // helper is a small amount of glue over these + std::env::var.

    #[test]
    fn parse_seed_accepts_decimal() {
        assert_eq!(parse_seed("SEED", "42").unwrap(), 42);
    }

    #[test]
    fn parse_seed_accepts_hex_prefix() {
        assert_eq!(parse_seed("SEED", "0xDEAD_BEEF").unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn parse_seed_accepts_underscore_separators() {
        assert_eq!(parse_seed("SEED", "1_000_000").unwrap(), 1_000_000);
        assert_eq!(
            parse_seed("SEED", "0xDEAD_BEEF_CAFE").unwrap(),
            0xDEAD_BEEF_CAFE
        );
    }

    #[test]
    fn parse_seed_reports_invalid_value_with_context() {
        let err = parse_seed("SEED", "not-a-number").unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("SEED"),
            "var name must be in the message: {msg}"
        );
        assert!(
            msg.contains("not-a-number"),
            "raw value must be in the message: {msg}"
        );
        assert!(
            msg.contains("0x"),
            "prefix example must be in the message: {msg}"
        );
    }

    #[test]
    fn time_seed_returns_recent_nanos() {
        let a = time_seed();
        let b = time_seed();
        // The clock should tick between two consecutive calls on any
        // realistic platform; if it doesn't, at least one of the two
        // must equal the other while both stay non-zero — the point of
        // the assertion is that the fallback isn't hard-coded to 0.
        assert!(a != 0 || b != 0);
    }

    // Smoke test the public helper's fallback path against a variable
    // name that is astronomically unlikely to be set in the test
    // environment. This exercises env::var(NotPresent) → time_seed.
    #[test]
    fn seed_from_env_or_time_falls_back_when_variable_unset() {
        let name = "NOPROP_SEED_TESTS_ABSOLUTELY_UNSET_9F3A_2E7B";
        let a = seed_from_env_or_time(name).expect("unset var must fall back");
        let b = seed_from_env_or_time(name).expect("unset var must fall back");
        // Two calls should both return a value (may or may not differ
        // depending on clock resolution). Non-zero-ness of at least one
        // is what we care about.
        assert!(a != 0 || b != 0);
    }
}
