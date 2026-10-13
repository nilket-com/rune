# C1 built-in assertion macro stock oracle

Base: 0a02c809. Production admission unchanged while this table is frozen.
178 cases: 74 values, 104 runtime errors; all cases compile on stock.
Fresh baseline: 178 FrontendDiagnostic macro-certification refusals.

The six TSV fields bind label, source, full stock observation, stdout, stderr,
and stock first-error source ID/start/end. Hex fields are UTF-8 bytes, not escaped
strings. Sites are collected during the same child invocation as the observation;
no second execution is used to infer operand effects. Existing frozen observations
are unchanged. The opt-in harness mode `assert-main` calls main, including a
private #[test] main, through normal entry lookup.

The bounded children use the existing S0 limits: 2M fuel, 10s wall, 128KiB streams.
The libtest preamble is retained in stdout; the child exits before its footer.
The stock replay checks source, exact observation, both streams, and first site.

Cases cover assert/assert_eq/assert_ne with plain, literal and formatted messages,
trailing commas; scalars, mixed numerics, NaN, strings and structural collections;
non-bool conditions; once-only left-to-right operands and failure-only message
arguments; operand/message errors; aliases mutated by later operands or message
arguments; a 2KiB Unicode message; depth62..65 for five container kinds; #[test]
entries; and fully qualified/aliased resolution.

Observed semantics:
- Message arguments run only if the assertion condition/comparison fails.
- assert_eq/assert_ne retain operands, evaluate them once left to right, and render
  their current values after evaluating custom message arguments.
- assert!(1) performs unary ! first, then fails a boolean type check; float/string/
  Unit/Vec instead fail the unary ! operation. The caret covers the invocation.
- Mixed signed/unsigned equality succeeds for1/1u64; NaN is unequal to itself.
- Depth64 and65 comparisons fail MaxValueDepth for all five container forms.
- At depth63 self-equality succeeds, but failure rendering for assert_ne raises
  MaxValueDepth for Vec/Tuple/Option/Result; Object still renders its panic text.

Fresh default FormatLimits.depth32 is a deliberate resource policy. Deep failure
rendering needs an explicit, reviewed retained-policy contract and a separately
configured-depth proof; these stock rows are never edited to conceal that policy
or any independent formatter semantic difference. Panic class labels in the stock
observation include the stock Debug payload; a future fresh contract may map only
that exact class spelling to fresh Panic, without weakening text/site checks.
