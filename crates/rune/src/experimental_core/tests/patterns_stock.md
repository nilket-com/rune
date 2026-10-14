# C3 pattern stock oracle

The frozen table contains 348 stock-only rows: the original 328 plus separately reviewed 14-row and six-row supplements. The raw runs remain local; this repository contains source cases, exact expectations and their bounded replay contract. The table records stock observations; the separate fresh contract below tests C3a admission.

`gap_hunt::patterns::stock_contract` replays every row in a separate bounded child with the default Context: 2M stock instruction budget, 10 seconds, 128KiB per output stream. The 63 explicit fuel rows instead use their recorded stock budgets. Every run requires a new GAP_OUT directory. The test verifies the actual engine and row count through the existing contract footer; `all_frozen_contracts` includes this stock contract and the separate fresh contract. Fresh fuel is a different accounting scheme and no numeric parity is claimed.

The table schema freezes label, mode, full source bytes (hex), the complete five-field stock observation (hex), raw stdout/stderr, runtime error source site (hex), compiler diagnostics including warnings (hex), and the Exact policy. The replay verifies every field, source order and the 348-row count. All recorded policies are Exact; no address, hash-order, failure-kind or message normalization is performed. An address observation would need explicit review; it is not silently admitted as equality.

Observed outcomes: 265 values, 74 runtime errors and 9 compile errors. These counts are stock observations, not coverage gains. Stock compile failures are excluded from eligibility/unlock denominators.

Eight ORIGINAL fixture rows contain bare named struct literals as the match scrutinee:

- struct/full/p, struct/full/q
- struct/partial/p, struct/partial/q
- struct/ignore/p, struct/ignore/q
- struct/nested/p, struct/nested/q

Their parser diagnostics are retained as fixture ambiguity, not stock language limitations. The eight corresponding `supplement/parenthesized/struct/...` rows add only scrutinee parentheses and supersede those originals for coverage. All eight corrected rows compile: P matches and the distinct nominal Q misses. The ninth compile error is `deferred/float`, intentionally UnsupportedPatternExpr.

The original `access/option/...` rows use unwrap(), which does not establish that the Option wrapper has moved. They remain operation observations. The supplement's four `dropped-option` rows use drop(a) and an alias: Some(_) and None report AccessConflict; a scalar literal misses without reading the payload; top-level _ succeeds. Moved String/Vec same-kind patterns also error, whereas scalar mismatches miss. Nested wildcards project and discard without reading the projected child's payload.

Observed tuple recipes: () matches Unit; (..) matches Unit OR any Tuple; nonempty tuple prefixes require Tuple and Len >= prefix length; closed nonempty tuples require exact length. A host-created zero-length fresh heap Tuple remains a separate fresh-only safety case.

Refutable let is DEFERRED for C3a. The stock mismatch has the typed class Panic { reason: UnmatchedPattern }, text `Panicked: pattern did not match`, the exact pattern span, and LetPatternMightPanic warning. Even the matching struct construction emits that warning. Existing fresh Op::Panic produces an owned message panic, not a typed unmatched-pattern reason. No generic S0 Panic alias or message inference is authorized.

C3a covers wildcard/identifier patterns, guarded scalar/string literals, Unit/tuple/Vec patterns, nominal structs, Some/None, admitted constant expansions, guards, match fallthrough to Unit, if-let and while-let. Mixed matches are refused whole if any pattern is outside that set. Refutable let, Result patterns, anonymous object patterns, user enums, tuple structs and float patterns remain deferred. Existing operations only; no executor/codec/layout/heap-accessor changes.

Stock binding lifetime is the implementation default. W1 uses pre-existing accessible child/container aliases, never a binding from another arm's lexical scope. These finite observations do not establish a general early-release equivalence. Invalid fresh handles, fresh heap-empty tuples, exact new fresh fuel frontiers and cleanup Miri remain separate safety/invariant tests, not fabricated stock parity rows.

Moved-ness is runtime state. For every admitted kind, stock AccessConflict class/text/site on a moved scrutinee must be reproduced; lowering cannot refuse only its moved cases. If the existing-operation recipe cannot do so, withdraw the whole pattern kind. Allocation/format limits never excuse an ordinary semantic divergence.

The final six rows settle address semantics: a bare Variable scrutinee is aliased and re-read after guard rebinding; a variable-result Block (including an effectful Block) is snapshotted and evaluated once. A while-let identifier binding copies the current value rather than aliasing the source variable. These distinctions are per HIR expression class; unrelated classes need stock source justification or refusal.

The two moved-Tuple rows show that both () and (..) must access the Tuple payload before any eventual length decision. The existing Len operation preserves that read and its AccessConflict text/site. The implementation's complete recipe for () is Unit OR (Tuple with Len == 0), including valid host-created empty heap tuples; this explicitly supersedes the earlier source-only Is Unit shorthand. For (..), Unit matches directly and heap Tuple requires Len for access, even though no length comparison is needed.

The unified gate and focused patterns_stock_count_hook test share one descriptor. The focused test checks its actual frozen-table count and emits the stock/348 footer; this validates the integration count hook without rerunning unrelated contracts. Earlier full-check evidence remains labelled with its 342-row source checkpoint.

## Fresh contract and limits

`gap_hunt::patterns::fresh_contract` runs the same 348 sources in bounded children: 248 compare stock class, text, value, streams and runtime site exactly; 37 assert an explicit refusal; 63 stock-fuel sources check fresh semantics at a full fresh budget. The stock budgets are never reused as fresh budgets. The 37 refusals comprise eight original ambiguous struct fixtures, ten deferred refutable-let cases, eleven other deferred pattern cases, seven pre-existing `unwrap()` method gaps and one pre-existing aggregate-constant frontend gap. The hard-coded case policy in the test identifies each refusal.

`patterns_safety` separately checks host-created empty and moved heap tuples, malformed access primitives, binding survival, cleanup on guard exits, and exact fresh fuel frontiers with balanced store/executor state. Its wide decision tree crosses a gap charge; other witnesses cover literal/Option/struct patterns, repeated false guards and repeated `while let` evaluation. These are fresh invariants, not stock instruction-budget parity. Deep pattern compilation is checked in isolated 2 MiB-stack children: the tested depth-126 shape succeeds, while depths 127 and 128 refuse with `DepthLimit`.
