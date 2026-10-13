# C2 stock output and panic oracle

Frozen on the production tree of `3f0bb1a5`, before C2 admission changes.
The oracle is test-only. `output_macros::stock_contract` requires both
`capture-io` and `disable-io`, a new `GAP_OUT`, and bounded subprocesses.

There are 367 rows: 351 exact stock replays and 16 explicitly named
`Observation(address-fallback)` rows. Raw streams and addresses are preserved.
For those 16 rows the replay compares every byte outside the one address field,
plus source, context, phase, result, error/site and stderr. It neither replaces
addresses globally nor claims exact-address or fresh semantic equivalence.
Range wrappers, moved/leased values and formatter-depth failures have distinct
causes despite sharing the observation label. Future fresh dispositions must
separately name formatting limits, fallback semantics and retained Debug policy.

The table includes 243 stock budget witnesses (0 through 80 for each macro).
They pin stock evaluation/output frontiers and exact failure sites; they do not
establish that the stock VM and fresh executor use interchangeable fuel units.
Fresh fuel semantics must additionally preserve the existing charged-op contract
and prove that no Debug output occurs before all arguments finish evaluating.

Context rows distinguish process stdout from `captured_hex`. Capture and disabled
handlers are alternate installs into `Context::with_config(false)`. The missing
IO profile has no print or dbg callee. Panic remains present in all profiles.
The non-finite float rows use explicitly injected scalar arguments, not source
constant admission. The four `host-fixture` rows use a test-only seed handler;
these do not establish fresh host-call support. The ordinary `moved/option` and
`lease/vec` rows are attempts: stock observes `Some([1])` and `[1]`, respectively,
without a moved/leased Debug failure. The host fixtures provide actual moved,
closed and exclusive-borrowed strings, plus a failing custom DEBUG_FMT handler.

The seven Linux descriptor rows are frozen separately. Broken peers produce
printing panics, including preserved prior output; a deliberately closed stdout
descriptor returns successfully in stock's Rust IO path. The partial-line row
shows `print!` buffering without an added flush; the fixture performs teardown
flush explicitly. Descriptor isolation, bounded handshakes and exact source
error spans are checked by the fixture. No product output policy changed.

Existing C1 stock rows (178) replayed unchanged after extracting the test-only
context/input seam. Existing default observations retain default context, no
injected argument and the same fuel. All frozen tables from prior records remain
unchanged. Production macro certificates and opcode/native admission are
unchanged by this oracle commit.
