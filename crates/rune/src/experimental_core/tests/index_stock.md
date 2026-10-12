# Frozen stock indexing contract

The table in `index_stock.tsv` records 1,035 bounded stock Rune runs before
S-index production changes. The generator and ignored contract test live in
`gap_hunt.rs`. Every source has its own child with 2M fuel, 10 seconds wall time,
and 128 KiB per output stream. Ordinary test runs do not launch this study.

The matrix covers 21 receivers, 12 dynamic indices with get/set/compound-add,
and four tuple-field indices with those three operations (1,008 cases).
Another 27 cases witness target/index/RHS order and short-circuiting on faults.
Abrupt evaluation uses division by zero, admitted by both frontends.

Columns are case label, source, observation, stdout, stderr. All except the
label are UTF-8/byte hex. Observations retain phase, error class, site, message,
and value. The contract compares phase/class/message/value and both streams
exactly; stock and fresh runtime site labels differ, and source spans have
focused diagnostic tests. The libtest child prefix is retained in stdout.
No error-message, value, or stream normalization is allowed.

Error classes are the existing explicit stock enum mapping, extended with
UnsupportedTupleIndexGet/Set -> Type and ValueToIntegerCoercionError ->
IntegerCoercion. This avoids unstable Debug function addresses. In S0 any
changed class labels must be reported separately from semantic convergence.

Notable stock distinctions pinned here:

- Dynamic get converts inline indices before checking the target. Dynamic set
  dispatches by target protocol; wrong targets can fail before conversion.
- Dynamic Tuple numeric set and compound-add fail, even when get succeeds.
- Err(value).0 get and compound-add succeed; .0 assignment fails. .1 assignment
  succeeds, while .1 get and compound-add fail.
- Unit tuple-field get reports MissingIndexInteger, while compound-add reports
  UnsupportedTupleIndexGet. Tuple-field set failures use UnsupportedTupleIndexSet.
- Out-of-range Vec tuple-field get/compound reports OutOfRange, while plain set
  reports UnsupportedTupleIndexSet.
- Object String-key assignment inserts missing keys. Struct missing fields fail.

The stock baseline uses the fork at d86a2075 with test-only changes. Raw
operator logs and per-child receipts remain local under
`.reviews/0185-s-index-stock3`; earlier attempts are retained separately.
