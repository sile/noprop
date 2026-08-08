//! Environment-variable helpers for populating [`Runner::seed`] and
//! `iterations`.
//!
//! The helpers are opt-in and are read only when the caller invokes
//! them, so the "no implicit I/O" contract of the rest of the crate is
//! preserved — `TestCaseContext::new` and `Runner::run` never touch the environment
//! or the clock on their own.
//!
//! The [`Runner::seed`](crate::Runner) rustdoc shows the intended
//! calling shape.

use std::env;
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::TestResult;

/// Parse `raw` as a seed or iteration count.
///
/// Accepts a plain decimal value, or a `0x` / `0b` / `0o` prefixed
/// value with optional `_` separators (e.g. `0xDEAD_BEEF`, `1_000_000`),
/// so the hex seed printed by failure reports can be pasted into an
/// environment variable directly.
fn parse_number<T>(var: &str, raw: &str) -> TestResult<T>
where
    T: FromNumber,
{
    let trimmed = raw.trim();
    let (radix, digits) = if let Some(rest) = trimmed.strip_prefix("0x") {
        (16, rest)
    } else if let Some(rest) = trimmed.strip_prefix("0b") {
        (2, rest)
    } else if let Some(rest) = trimmed.strip_prefix("0o") {
        (8, rest)
    } else {
        (10, trimmed)
    };
    let cleaned: String = digits.chars().filter(|c| *c != '_').collect();
    T::from_str_radix(&cleaned, radix).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "environment variable {var:?} has an invalid value {raw:?}: {err}; \
                 expected a decimal integer or a 0x / 0b / 0o prefixed value (e.g. 0xDEAD_BEEF, 1_000_000)"
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
/// - `var` set to a valid `u64` (decimal, or `0x` / `0b` / `0o`
///   prefixed, with optional `_` separators) — returns that value.
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
/// let _ = noprop::Runner::new(seed, 256);
/// ```
pub fn seed_from_env_or_time(var: &str) -> TestResult<u64> {
    match env::var(var) {
        Ok(raw) => parse_number(var, &raw),
        Err(env::VarError::NotPresent) => Ok(time_seed()),
        Err(env::VarError::NotUnicode(_)) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("environment variable {var:?} is not valid UTF-8"),
        )
        .into()),
    }
}

/// Read `var` as a `usize` iteration count, or return `default` when it
/// is not set.
///
/// Behavior:
///
/// - `var` unset (`NotPresent`) — returns `default`.
/// - `var` set to a valid `usize` (decimal, or `0x` / `0b` / `0o`
///   prefixed, with optional `_` separators) — returns that value.
/// - `var` set to a value that fails to parse — returns a boxed
///   [`io::Error`] naming the variable, the raw value, and the parse
///   error. `default` is **not** silently substituted;
///   misconfiguration surfaces as an error.
/// - `var` set to a non-Unicode value — returns a boxed [`io::Error`]
///   naming the variable.
///
/// # Examples
///
/// ```
/// let iterations = noprop::iterations_from_env("MYAPP_ITERATIONS", 256)
///     .expect("MYAPP_ITERATIONS, if set, must parse as usize");
/// let _ = noprop::Runner::new(0, iterations);
/// ```
pub fn iterations_from_env(var: &str, default: usize) -> TestResult<usize> {
    match env::var(var) {
        Ok(raw) => parse_number(var, &raw),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("environment variable {var:?} is not valid UTF-8"),
        )
        .into()),
    }
}

/// Numbers the env helpers can parse from a string.
trait FromNumber: Sized {
    fn from_str_radix(src: &str, radix: u32) -> Result<Self, std::num::ParseIntError>;
}

impl FromNumber for u64 {
    fn from_str_radix(src: &str, radix: u32) -> Result<Self, std::num::ParseIntError> {
        u64::from_str_radix(src, radix)
    }
}

impl FromNumber for usize {
    fn from_str_radix(src: &str, radix: u32) -> Result<Self, std::num::ParseIntError> {
        usize::from_str_radix(src, radix)
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
    // process-wide state: parse_number for the accepted forms and
    // error kinds, and time_seed for the fallback path. The public
    // helpers are a small amount of glue over these + std::env::var.

    #[test]
    fn parse_number_accepts_decimal() {
        let v: u64 = parse_number("SEED", "42").unwrap();
        assert_eq!(v, 42);
    }

    #[test]
    fn parse_number_accepts_hex_prefix() {
        let v: u64 = parse_number("SEED", "0xDEAD_BEEF").unwrap();
        assert_eq!(v, 0xDEAD_BEEF);
    }

    #[test]
    fn parse_number_accepts_binary_and_octal_prefixes() {
        let b: u64 = parse_number("SEED", "0b1010").unwrap();
        assert_eq!(b, 0b1010);
        let o: u64 = parse_number("SEED", "0o17").unwrap();
        assert_eq!(o, 0o17);
    }

    #[test]
    fn parse_number_accepts_underscore_separators() {
        let v: u64 = parse_number("SEED", "1_000_000").unwrap();
        assert_eq!(v, 1_000_000);
        let h: u64 = parse_number("SEED", "0xDEAD_BEEF_CAFE").unwrap();
        assert_eq!(h, 0xDEAD_BEEF_CAFE);
    }

    #[test]
    fn parse_number_accepts_valid_usize() {
        let v: usize = parse_number("ITER", "128").unwrap();
        assert_eq!(v, 128);
    }

    #[test]
    fn parse_number_reports_invalid_value_with_context() {
        let err = parse_number::<u64>("SEED", "not-a-number").unwrap_err();
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
            msg.contains("0x") && msg.contains("0b"),
            "prefix examples must be in the message: {msg}"
        );
    }

    #[test]
    fn parse_number_reports_negative_for_usize() {
        let err = parse_number::<usize>("ITER", "-1").unwrap_err();
        assert!(err.to_string().contains("ITER"));
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
        let name = "NOPROP_CONFIG_TESTS_ABSOLUTELY_UNSET_SEED_9F3A_2E7B";
        let a = seed_from_env_or_time(name).expect("unset var must fall back");
        let b = seed_from_env_or_time(name).expect("unset var must fall back");
        // Two calls should both return a value (may or may not differ
        // depending on clock resolution). Non-zero-ness of at least one
        // is what we care about.
        assert!(a != 0 || b != 0);
    }

    #[test]
    fn iterations_from_env_uses_default_when_variable_unset() {
        let name = "NOPROP_CONFIG_TESTS_ABSOLUTELY_UNSET_ITER_9F3A_2E7B";
        let v = iterations_from_env(name, 512).expect("unset var must use default");
        assert_eq!(v, 512);
    }
}
