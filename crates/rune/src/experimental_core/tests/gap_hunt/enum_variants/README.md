# C15b enum and match oracle

The oracle freezes 166 inputs before implementation: 142 Exact, 4 Intentional declaration-order comparisons, 10 frontend refusals, 8 surface refusals, 1 depth refusal and 1 retained fuel rule. Authority: chatd messages 18976, 18981 and 18997. The base is C15 commit 9e4caecf.

The original 163 inputs and their observations are unchanged. Rows 058, 059 and 123 preserve the frontend diagnostics for a named constructor used directly as a match scrutinee. New rows 163 and 164 bind that value before matching and return 7 and 16. New row 165 reports the unknown pattern field. The appended inputs use the second stock VM binary; all original observations use the first binary. The external review package retains child receipts and source provenance.

`stock_raw.json` preserves the original observations. `stock.json` omits raw debug evidence and projects only the seven approved structured kind leaves containing nine `AnyTypeInfo.full_name` pointers. All hashes, type names, spans, diagnostics, effects, reuse results and value graph identities remain exact. `p1.json` lists the exact leaves.

`intentional.json` records declaration-order comparison in both engines for rows 036, 039, 040 and 052. Each rule retains the stock-before observation alongside the approved expectation. Only row 036 changes output for these inputs; the other three happen to agree with the old hash order. These expectations follow the approved language rule and are not measured implementation results. No older oracle input found by the enum/relational-source audit pins a different variant order.

Row 148 has a source-identical stock VM completion at fuel 200000 with 100 events. A fresh core fuel stop must retain a non-strict prefix of that complete tape, close its lifecycle and allow executor reuse. Completion must match the complete observation. This rule checks effect order, not equal numeric fuel consumption.

The scope includes unit, empty tuple, tuple and named variants, enum identity, fields, formatting, equality, declaration-order comparison, match, if-let and while-let. Nested patterns, guards, rest, wrong-type misses and unit on no arm retain the observed behavior. First-class constructors, refutable local and parameter patterns, bare named-variant type values and async blocks remain compile-time refusals before effects. Depth 128 remains a compile-time refusal. The nominal clone aliasing defect is reserved for C15c; its observations are preserved here.

The ignored stock VM child requires an external bounded operator. Observer and replay controls check source and file hashes, projection, value/type/alias sensitivity, intentional expectations and the complete fuel tape. The fresh core contract will accompany implementation. This commit changes only tests and oracle data.
