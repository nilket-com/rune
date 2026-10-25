C14 stock oracle: string text methods

The 190 source rows cover String split/replace/starts_with/ends_with/contains/trim,
the String/char Split iterator surface, refusals, and ownership boundaries.
Stock observations are immutable. `stock_raw.json` is the audit archive;
`stock.json` is the portable semantic projection. Only the enumerated
`full_name` pointer tokens in `p1.json` are replaced. No hashes, messages,
argument chains, graphs, event tapes or callsite spans are normalized.
`freeze.json` records the catalog digest and per-row binary/observation provenance.
The first 186 observations came from one collection; four approved ownership
witnesses were collected separately without replaying those original rows.

`dispositions.json` fixes 130 Exact, 24 Refused-delimiter, 12 Refused-arity,
4 Refused-surface, 18 Refused-provider and 2 Retained-fuel rows.
Refused calls are rejected before effects. Split delimiters must be direct
String/char literals, or resolved local bindings initialized by such a literal
with only direct same-kind literal writes throughout their function.
Parameters, alias/computed/joined initializers, captures, blocks and Function
delimiters are outside that proof. Arity mismatches are compile-time refusals.
Untrusted same-name providers close admission. No admitted runtime case may
return Unsupported; InvalidCode denotes only a broken internal invariant.
Retained-fuel means exhaustion with an empty tape, idle cleared Store and
reuse result 7; it does not promise equal instruction counts or allow success.

Split aliases share a cursor. Stock's empty-string/empty-delimiter result has
one empty element. The source is borrowed, the String delimiter is copied,
and the borrow is released on the final yield or iterator drop. `.iter()`
on Split is a missing-method error, distinct from trait iteration in a for loop.
Vec collection is limited to certified String/char Split origins. String
collection, skip/take/count, chars, unwrap_or and Function delimiters remain
outside C14. No whole-program admission is inferred from one removed blocker.

The stock child and observer controls run under `gap_hunt::text_methods`.
The independent execution contract lands with the implementation so this test-only
commit remains green before those methods are implemented.
