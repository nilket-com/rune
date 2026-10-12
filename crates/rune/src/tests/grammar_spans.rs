//! Empty recovery nodes point at the significant token after skipped whitespace.
prelude!();
use ErrorKind::*;

#[test]
fn empty_expression_error_spans_skip_whitespace() {
	assert_errors! { "let x = ;", span!(8, 8), ExpectedSyntax { .. } };
	assert_errors! { "let x =   ;", span!(10, 10), ExpectedSyntax { .. } };
	assert_errors! { "let x =", span!(7, 7), ExpectedSyntax { .. } };
	assert_errors! { "let x = ", span!(8, 8), ExpectedSyntax { .. } };
}

#[test]
fn consumed_recovery_keeps_its_token_span() {
	assert_errors! { "let x = ) ;", span!(8, 9), ExpectedSyntax { .. } };
}

#[test]
fn neighbouring_missing_token_spans_do_not_move() {
	assert_errors! { "  ;", span!(2, 2), ExpectedSyntax { .. } };
	assert_errors! { "let x = (1;", span!(10, 10), UnexpectedEndOfSyntaxWith { .. } };
	assert_errors! { "match 1 { 1 }", span!(11, 11), UnexpectedEndOfSyntaxWith { .. } };
}
