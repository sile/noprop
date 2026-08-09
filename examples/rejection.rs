//! Rejection: the two ways to discard input, and when to use which,
//! applied to parsing `key=value` configuration lines.
//!
//! - `sample_with_rejection(ctx, max_attempts, attempt)`: retry a
//!   *single constrained draw* (here: a key that is a valid
//!   identifier) up to `max_attempts` times. The attempt boundary
//!   stays in the choice sequence.
//! - `TestCaseContext::reject_case()`: discard the *whole case* when
//!   its preconditions are violated after sampling (here: a comment
//!   or blank line is not a configuration record at all). Rejected
//!   cases are retried and do not count toward the case budget.
//!
//! Run with: `cargo run --example rejection`

/// Parse a `key=value` configuration line. The key must be a valid
/// identifier; the value must fit in `u32`.
fn parse_pair(line: &str) -> Option<(&str, u32)> {
    let (key, value) = line.split_once('=')?;
    if !is_identifier(key) {
        return None;
    }
    let value: u32 = value.parse().ok()?;
    Some((key, value))
}

/// An identifier: ASCII alphanumeric or underscore, not starting with
/// a digit.
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn main() -> noprop::TestResult {
    // === Constrained draw: sample_with_rejection ===
    //
    // The property needs a valid identifier as the key. Keeping the
    // rejection local to the draw retries only the key — regenerating
    // the whole case would redraw the value too.
    let mut runner = noprop::Runner::new(0xDEAD_BEEF);
    runner.run(64, |ctx| {
        let key = noprop::sample_with_rejection(ctx, 16, |ctx| {
            let k = noprop::sample_ascii_string(ctx, 2);
            is_identifier(&k).then_some(k)
        });
        let value = noprop::sample_u32(ctx);
        let line = format!("{key}={value}");
        let (parsed_key, parsed_value) = parse_pair(&line).expect("the generated pair must parse");
        assert_eq!(parsed_key, key);
        assert_eq!(parsed_value, value);
        Ok(())
    })?;
    println!(
        "constrained draw: passed (rejected {} cases)",
        runner.stats().rejected_cases
    );

    // === Whole-case precondition: reject_case ===
    //
    // The property only studies configuration records: a line
    // containing a `#` comment is not a record at all, so the case is
    // rejected after sampling. Rejections are bounded — an
    // always-invalid generator still terminates with a
    // TooManyRejections error.
    let mut runner = noprop::Runner::new(0xFEED);
    runner.run(32, |ctx| {
        let line = noprop::sample_ascii_string(ctx, 12);
        if line.is_empty() || line.contains('#') {
            ctx.reject_case();
        }
        // A non-comment line may still be malformed: parse_pair must
        // either succeed with a valid identifier or fail cleanly.
        if let Some((key, _)) = parse_pair(&line) {
            assert!(is_identifier(key), "parsed key must be an identifier");
        }
        Ok(())
    })?;
    println!(
        "whole-case rejection: passed (rejected {} cases)",
        runner.stats().rejected_cases
    );
    Ok(())
}
