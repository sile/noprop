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
use std::num::ParseIntError;
use std::time::{SystemTime, UNIX_EPOCH};

/// Failure returned by [`seed_from_env_or_time`] and
/// [`iterations_from_env`] when the environment variable is present but
/// unusable.
///
/// The helpers deliberately do **not** silently fall back on a set-but-
/// broken value: a mistyped `MYAPP_SEED=hello` should surface as an
/// error rather than as "well, we just used the clock", so CI runs stay
/// reproducible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// The variable was set but its contents were not valid UTF-8.
    InvalidUnicode { var: String },
    /// The variable was set to a value that could not be parsed as the
    /// expected numeric type.
    InvalidValue {
        var: String,
        raw: String,
        message: String,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::InvalidUnicode { var } => {
                write!(f, "environment variable {var:?} is not valid UTF-8")
            }
            ConfigError::InvalidValue { var, raw, message } => {
                write!(
                    f,
                    "environment variable {var:?} has an invalid value {raw:?}: {message}"
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// Read `var` as a `u64` seed, or derive one from the current time when
/// it is not set.
///
/// Behavior:
///
/// - `var` unset (`NotPresent`) — returns a seed derived from
///   `SystemTime::now() - UNIX_EPOCH` in nanoseconds, cast to `u64`.
///   If the system clock is before the Unix epoch the fallback is `0`
///   (still a legitimate seed for the internal PRNG).
/// - `var` set to a valid `u64` — returns that value.
/// - `var` set to a value that fails to parse as `u64` — returns
///   [`ConfigError::InvalidValue`].
/// - `var` set to a non-Unicode value — returns
///   [`ConfigError::InvalidUnicode`].
///
/// # Examples
///
/// ```
/// let seed = noprop::seed_from_env_or_time("MYAPP_SEED")
///     .expect("MYAPP_SEED, if set, must parse as u64");
/// let _ = noprop::Runner::new(seed, 256);
/// ```
pub fn seed_from_env_or_time(var: &str) -> Result<u64, ConfigError> {
    match env::var(var) {
        Ok(raw) => parse_number(var, &raw),
        Err(env::VarError::NotPresent) => Ok(time_seed()),
        Err(env::VarError::NotUnicode(_)) => Err(ConfigError::InvalidUnicode {
            var: var.to_string(),
        }),
    }
}

/// Read `var` as a `usize` iteration count, or return `default` when it
/// is not set.
///
/// Behavior:
///
/// - `var` unset (`NotPresent`) — returns `default`.
/// - `var` set to a valid `usize` — returns that value.
/// - `var` set to a value that fails to parse as `usize` — returns
///   [`ConfigError::InvalidValue`]. `default` is **not** silently
///   substituted; misconfiguration surfaces as an error.
/// - `var` set to a non-Unicode value — returns
///   [`ConfigError::InvalidUnicode`].
///
/// # Examples
///
/// ```
/// let iterations = noprop::iterations_from_env("MYAPP_ITERATIONS", 256)
///     .expect("MYAPP_ITERATIONS, if set, must parse as usize");
/// let _ = noprop::Runner::new(0, iterations);
/// ```
pub fn iterations_from_env(var: &str, default: usize) -> Result<usize, ConfigError> {
    match env::var(var) {
        Ok(raw) => parse_number(var, &raw),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(ConfigError::InvalidUnicode {
            var: var.to_string(),
        }),
    }
}

/// Common parse path for both helpers. Turns the raw string into `T`
/// (`u64` for seeds, `usize` for iterations) and wraps a parse failure
/// in [`ConfigError::InvalidValue`] with the offending value and the
/// standard-library parse message attached.
fn parse_number<T>(var: &str, raw: &str) -> Result<T, ConfigError>
where
    T: std::str::FromStr<Err = ParseIntError>,
{
    raw.parse::<T>().map_err(|err| ConfigError::InvalidValue {
        var: var.to_string(),
        raw: raw.to_string(),
        message: err.to_string(),
    })
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
    // process-wide state: parse_number for the two error kinds, and
    // time_seed for the fallback path. The public helpers are a small
    // amount of glue over these + std::env::var.

    #[test]
    fn parse_number_accepts_valid_u64() {
        let v: u64 = parse_number("SEED", "42").unwrap();
        assert_eq!(v, 42);
    }

    #[test]
    fn parse_number_accepts_valid_usize() {
        let v: usize = parse_number("ITER", "128").unwrap();
        assert_eq!(v, 128);
    }

    #[test]
    fn parse_number_reports_invalid_value_with_context() {
        let err = parse_number::<u64>("SEED", "not-a-number").unwrap_err();
        match err {
            ConfigError::InvalidValue { var, raw, message } => {
                assert_eq!(var, "SEED");
                assert_eq!(raw, "not-a-number");
                assert!(!message.is_empty(), "parse message should be populated");
            }
            other => panic!("expected InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn parse_number_reports_negative_for_usize() {
        let err = parse_number::<usize>("ITER", "-1").unwrap_err();
        assert!(matches!(err, ConfigError::InvalidValue { .. }));
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

    #[test]
    fn config_error_display_mentions_the_variable() {
        let err = ConfigError::InvalidUnicode {
            var: "MYAPP_SEED".into(),
        };
        assert!(err.to_string().contains("MYAPP_SEED"));

        let err = ConfigError::InvalidValue {
            var: "MYAPP_ITERATIONS".into(),
            raw: "abc".into(),
            message: "invalid digit found in string".into(),
        };
        let text = err.to_string();
        assert!(text.contains("MYAPP_ITERATIONS"));
        assert!(text.contains("abc"));
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
