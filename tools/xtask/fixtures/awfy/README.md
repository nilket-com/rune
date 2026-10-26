These small frozen oracle and diagnostic files are byte-identical copies of the
completed S7b2 review inputs. Their digests and relative repository paths are in
`../../manifests/awfy.json`. AWFY program inputs already live in `benches/awfy`;
the verifier reads those files directly and verifies their frozen digests.

The copied `run.py` and `src/main.rs` are historical defect evidence, never
executed or built by xtask. Absolute names in frozen observations and logical
source origins are historical data, never filesystem read paths or commands.
No external fixture directory is required for this suite. The six newly active
controls have separate frozen first-run expectations in
`ACTIVE-FIRST-OBSERVATIONS.json`. Its original pending-freeze status records the
first diagnostic run, not a final acceptance result. The final verifier binds all
six exact names, environments and wire tuples to that file by digest. Historical
inactive observations remain unchanged and carry no resource-limit claim.
