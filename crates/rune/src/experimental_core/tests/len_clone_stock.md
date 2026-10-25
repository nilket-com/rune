# S-len/clone stock oracle

Frozen against stock Rune at parent `2817424579bf702728baf88b33ccf105052eb957`.
Production semantics are unchanged by this checkpoint.

310 bounded source cases: 175 method/argument-order cases on 25 receivers,
32 clone witnesses, and 103 alias/invalidation cases. Each uses the existing
S0 child harness: 2 million fuel, 10 seconds wall, 128 KiB per output stream.
The TSV stores source, complete observation, stdout and stderr as hex. Scalar
observations include their type. No object iteration or Debug order is normalized.
Multi-key Object method rows observe only their count or method error.

`len_clone::legacy_contract` checks exact frozen sources, outcomes and streams;
run explicitly with `--ignored --exact` and a new `GAP_OUT`. `legacy_probe` and
`probe` write inventories into new directories. Execution conformance and
coverage refusals are not asserted at this tests-first checkpoint. Existing
field/index/S0 fixtures stay frozen and unchanged.

The stock table establishes:

- Aliasing is live until explicit `drop` invalidates the shared payload. Debug,
  Display, method resolution, reads, writes, double drop, nested invalidation,
  and reassignment are observed separately; unsupported operations can take
  precedence over access to an invalidated payload. Read, Write, and Take
  (double-drop) are distinct stock moved causes. Passing an invalidated value
  without reading it remains successful.
- Supported methods with an extra argument fail at runtime with ArgumentCount;
  unsupported receivers can fail method resolution first. RHS output is retained.
- Tuple/Option/Result clone recursively; Vec/Object copy only their outer storage;
  nested structs remain aliases. Direct struct clone misses the method.
- Function clone is callable; stock Function equality is unsupported.
- Repeated DAG occurrences clone independently. Shallow Vec/Object cycles succeed
  and are observed only via scalar lengths, avoiding recursive formatting.
- At depths 1, 2, 8 and 64, Some/Ok/Err clone and mutation produce `(1, 2)` leaf
  lengths. This observes copy semantics without invoking Debug's separate depth cap.

Recursive-cycle rejection and scratch/result budget rollback belong in execution-only
heap tests. Deep stock aborts are not repeated. Any 100k execution stress stays opt-in
and child-bounded, outside the ordinary suite and Miri.

A first frozen-contract check found that Object Debug of an invalidated child
contains an ASLR-dependent address. The object/nested-parent row therefore
observes parent length; the separate nested-child row observes the access error.
The original raw table and failed check are retained, not normalized.

Local raw receipts: `/home/me/work/.reviews/0185-s-len-clone-stock3`.
The initial discovery run (`...stock1`) is preserved: it used the old generic
BadArgumentCount classifier and formatted nested values directly, encountering
Debug's depth cap at depth 64. The final table uses explicit ArgumentCount and
leaf observations. No production edits occurred between these runs.
