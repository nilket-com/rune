# Frozen stock comparison oracle

Measured against fork c875ab03571bdd1b41553f12a5f055cb7a5a2cc5, before S-eq production changes.
`comparison_stock.tsv` contains 3,432 primary rows plus 162 supplemental rows.
The generators in `gap_hunt.rs` define the executable sources by case id.
The original measurements used `f64::NAN` and inserted a new field to create an
object cycle. The current sources produce NaN with `0.0 / 0.0` and precreate the
cycle's field, avoiding unrelated new constant/missing-field limitations.
A full stock replay validates the unchanged frozen expectations on these sources. Local raw
sources, observations and streams are retained in `0185-s-eq-stock1` and
`0185-s-eq-stock-supplement2` under the campaign review directory.

Each source runs in an isolated stock child: 2,000,000 fuel, 10-second wall limit,
128 KiB per output stream. Every recorded child completed with a value or error;
there are no compile refusals, timeouts or missing observations in this table.
The full ordered type-pair matrix covers 23 values and six operators. Supplemental
cases cover prefix precedence, iterable RHS equality, cycles, sharing, NaN,
nominal types, and container depths 62–65.

`exact` rows compare phase, error class, exact rendered message and typed value.
The handful of explicitly tagged object equality/inequality rows permit only the
listed boolean or leaf Type error alternatives: stock hash iteration can encounter
a false child or an error first. Object ordering is always exact. These policies
never admit resource failures or unrelated Type errors. Source spans are covered
by focused diagnostics tests, rather than normalized by this table runner.

The probe and oracle tests are ignored in ordinary test runs. Invoke explicitly
with a new `GAP_OUT`, `GAP_COMPARE_ENGINE=stock` or `new`, and features
`fmt,workspace,execution,native-code`. Select `comparison_contract` and
`--ignored`. The ordinary metadata control test guards the exception policies.

This is a finite semantic oracle, not a performance claim or exhaustive proof.
