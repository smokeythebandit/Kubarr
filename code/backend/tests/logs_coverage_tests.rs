//! Unit tests for the `convert_loki_to_logsql` function in
//! `src/endpoints/logs.rs` and for the public log-related structs.
//!
//! `convert_loki_to_logsql` translates Loki LogQL queries into
//! VictoriaLogs LogsQL format.  The function is `pub(crate)` and is
//! re-exported here via the `kubarr` crate.
//!
//! Covered cases:
//! - Empty string → wildcard "*"
//! - Whitespace-only string → wildcard "*"
//! - Bare wildcard "*" → "*"
//! - Simple equality matcher {job="foo"} → job:foo
//! - Regex matcher {job=~"foo.*"} → job:~foo.*
//! - Negative equality {job!="foo"} → NOT job:foo
//! - Multiple matchers {job="foo",env="prod"} → joined with " AND "
//! - Matcher with no quotes (bare value) {job=foo}
//! - Empty braces {} → wildcard "*"
//! - Only commas inside braces {,} → wildcard "*"
//! - Non-brace query passes through as-is (plain LogsQL)
//! - Mixed operator order: != checked before =
//! - Special characters in values
//! - Whitespace around operators is trimmed

use kubarr::endpoints::logs::convert_loki_to_logsql;

// ============================================================================
// Empty / wildcard inputs
// ============================================================================

#[test]
fn empty_string_returns_wildcard() {
    assert_eq!(convert_loki_to_logsql(""), "*");
}

#[test]
fn whitespace_only_returns_wildcard() {
    // The function trims first, then checks for empty
    assert_eq!(convert_loki_to_logsql("   "), "*");
    assert_eq!(convert_loki_to_logsql("\t"), "*");
    assert_eq!(convert_loki_to_logsql("\n"), "*");
}

#[test]
fn bare_wildcard_returns_wildcard() {
    assert_eq!(convert_loki_to_logsql("*"), "*");
}

#[test]
fn wildcard_with_surrounding_whitespace_returns_wildcard() {
    assert_eq!(convert_loki_to_logsql("  *  "), "*");
}

// ============================================================================
// Simple equality matcher  {label="value"}
// ============================================================================

#[test]
fn simple_equality_matcher() {
    let result = convert_loki_to_logsql(r#"{job="foo"}"#);
    assert_eq!(result, "job:foo");
}

#[test]
fn simple_equality_matcher_strips_quotes() {
    // Quotes around the value must be stripped
    let result = convert_loki_to_logsql(r#"{namespace="media"}"#);
    assert_eq!(result, "namespace:media");
}

#[test]
fn equality_matcher_with_numeric_value() {
    let result = convert_loki_to_logsql(r#"{status="200"}"#);
    assert_eq!(result, "status:200");
}

// ============================================================================
// Regex matcher  {label=~"pattern"}
// ============================================================================

#[test]
fn regex_matcher() {
    let result = convert_loki_to_logsql(r#"{job=~"foo.*"}"#);
    assert_eq!(result, "job:~foo.*");
}

#[test]
fn regex_matcher_complex_pattern() {
    let result = convert_loki_to_logsql(r#"{pod=~"my-app-.*"}"#);
    assert_eq!(result, "pod:~my-app-.*");
}

#[test]
fn regex_matcher_strips_quotes() {
    // The quotes around the regex pattern should be stripped
    let result = convert_loki_to_logsql(r#"{container=~"nginx|apache"}"#);
    assert_eq!(result, "container:~nginx|apache");
}

// ============================================================================
// Negative equality matcher  {label!="value"}
// ============================================================================

#[test]
fn negative_equality_matcher() {
    let result = convert_loki_to_logsql(r#"{job!="foo"}"#);
    assert_eq!(result, "NOT job:foo");
}

#[test]
fn negative_matcher_strips_quotes() {
    let result = convert_loki_to_logsql(r#"{level!="debug"}"#);
    assert_eq!(result, "NOT level:debug");
}

// ============================================================================
// Multiple matchers  {label1="val1",label2="val2"}
// ============================================================================

#[test]
fn two_equality_matchers() {
    let result = convert_loki_to_logsql(r#"{job="foo",env="prod"}"#);
    assert_eq!(result, "job:foo AND env:prod");
}

#[test]
fn three_matchers() {
    let result =
        convert_loki_to_logsql(r#"{namespace="media",pod="jellyfin",container="jellyfin"}"#);
    assert_eq!(
        result,
        "namespace:media AND pod:jellyfin AND container:jellyfin"
    );
}

#[test]
fn mixed_operators_two_matchers() {
    let result = convert_loki_to_logsql(r#"{job="foo",level!="debug"}"#);
    assert_eq!(result, "job:foo AND NOT level:debug");
}

#[test]
fn regex_and_equality_matchers() {
    let result = convert_loki_to_logsql(r#"{namespace="media",pod=~"sonarr-.*"}"#);
    assert_eq!(result, "namespace:media AND pod:~sonarr-.*");
}

#[test]
fn multiple_matchers_order_preserved() {
    // Order of logsql_parts follows input order
    let result = convert_loki_to_logsql(r#"{a="1",b="2",c="3"}"#);
    assert_eq!(result, "a:1 AND b:2 AND c:3");
}

// ============================================================================
// Empty braces and comma-only content
// ============================================================================

#[test]
fn empty_braces_returns_wildcard() {
    // {} → inner is empty → logsql_parts is empty → return "*"
    let result = convert_loki_to_logsql("{}");
    assert_eq!(result, "*");
}

#[test]
fn braces_with_only_commas_returns_wildcard() {
    // {,} → both parts are empty after trim → skipped → return "*"
    let result = convert_loki_to_logsql("{,}");
    assert_eq!(result, "*");
}

#[test]
fn braces_with_whitespace_only_inner_returns_wildcard() {
    let result = convert_loki_to_logsql("{   }");
    // "   " is a single part that, when trimmed, is empty → skipped → "*"
    assert_eq!(result, "*");
}

// ============================================================================
// Non-brace queries pass through as-is
// ============================================================================

#[test]
fn plain_logsql_passthrough() {
    // A plain LogsQL expression without braces should be returned unchanged
    let result = convert_loki_to_logsql("namespace:media");
    assert_eq!(result, "namespace:media");
}

#[test]
fn complex_logsql_passthrough() {
    let query = "namespace:media AND pod:~sonarr-.*";
    let result = convert_loki_to_logsql(query);
    assert_eq!(result, query);
}

#[test]
fn plain_word_passthrough() {
    let result = convert_loki_to_logsql("error");
    assert_eq!(result, "error");
}

#[test]
fn leading_trailing_whitespace_trimmed_for_passthrough() {
    let result = convert_loki_to_logsql("  namespace:media  ");
    assert_eq!(result, "namespace:media");
}

// ============================================================================
// Bare value (no quotes around value)
// ============================================================================

#[test]
fn equality_matcher_bare_value_no_quotes() {
    // {job=foo} — no quotes; value is "foo" after trim
    let result = convert_loki_to_logsql("{job=foo}");
    assert_eq!(result, "job:foo");
}

// ============================================================================
// Whitespace around operators is handled
// ============================================================================

#[test]
fn whitespace_around_equals_in_matcher() {
    // The label and value are trimmed individually
    let result = convert_loki_to_logsql(r#"{job = "foo"}"#);
    assert_eq!(result, "job:foo");
}

#[test]
fn whitespace_between_matchers() {
    // Spaces after commas
    let result = convert_loki_to_logsql(r#"{job="foo", env="prod"}"#);
    assert_eq!(result, "job:foo AND env:prod");
}

// ============================================================================
// Special characters in values
// ============================================================================

#[test]
fn value_with_hyphen() {
    let result = convert_loki_to_logsql(r#"{pod="my-app-12345"}"#);
    assert_eq!(result, "pod:my-app-12345");
}

#[test]
fn value_with_underscore() {
    let result = convert_loki_to_logsql(r#"{container="my_container"}"#);
    assert_eq!(result, "container:my_container");
}

#[test]
fn value_with_dot() {
    let result = convert_loki_to_logsql(r#"{host="node.example.com"}"#);
    assert_eq!(result, "host:node.example.com");
}

#[test]
fn value_with_slash() {
    let result = convert_loki_to_logsql(r#"{path="/api/health"}"#);
    assert_eq!(result, "path:/api/health");
}

// ============================================================================
// Operator precedence — != must be detected before =
// ============================================================================

#[test]
fn negative_operator_not_confused_with_equals() {
    // The implementation checks "=~" and "!=" before plain "="
    // so {level!="debug"} must produce "NOT level:debug", not "level!:debug"
    let result = convert_loki_to_logsql(r#"{level!="debug"}"#);
    assert!(
        result.starts_with("NOT "),
        "must produce NOT prefix, got: {}",
        result
    );
    assert_eq!(result, "NOT level:debug");
}

#[test]
fn regex_operator_not_confused_with_equals() {
    // {job=~"foo"} must produce "job:~foo", not "job:~\"foo\""
    let result = convert_loki_to_logsql(r#"{job=~"foo"}"#);
    assert!(
        result.contains(":~"),
        "must produce :~ operator, got: {}",
        result
    );
    assert_eq!(result, "job:~foo");
}
