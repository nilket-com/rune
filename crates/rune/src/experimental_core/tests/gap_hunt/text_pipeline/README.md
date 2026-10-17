# C14b stock oracle: text pipeline

165 source-pinned stock observations collected once on C14 32bd6d4a. This commit is test-only; it does not admit fresh-core operations.

Closed origin set: StringSplit, CharSplit, Chars, Skip(O), Take(O). New skip/take and next/for/count/Vec-or-String collect are restricted to this set. Defaults for Option/Result.unwrap_or evaluate eagerly. Adapters preserve shared iterator identity and advance lazily.

Dispositions and the limited full_name pointer projection are frozen separately from raw observations. Exact means full projected outcome, events and VM reuse, including errors. Refused rows are compilation boundaries, not runtime Unsupported. Fuel rows use the rule recorded in freeze.json; high-fuel counterfactuals may be collected once per row only if fresh completes.

The ignored stock child requires an explicit external bounded operator. The ordinary stock_replay_controls test verifies catalogue, frozen hashes, projections and observation metadata without executing scripts. The fresh contract lands with the implementation so this commit stays green.
