# Are We Fast Yet, in Rune

Rune ports of the [Are We Fast Yet](https://github.com/smarr/are-we-fast-yet) benchmarks
(Marr, Daloze and Mössenböck, DLS 2016), made from the Lua versions at commit
`74306fec151070fd07157cefeacf19e7e0bcdc89`. They follow the project's
[guidelines](https://github.com/smarr/are-we-fast-yet/blob/master/docs/guidelines.md):
the same algorithms and code structure as every other language, using only the core
language (objects, polymorphic methods, closures, fixed-size arrays and strings) and the
shared `som` library instead of built-in collections. Licenses are in `NOTICE.txt`.

## Programs

A program is `harness.rn`, `benchmark.rn`, `som.rn` (when the benchmark uses it) and one
benchmark file, compiled together as one unit. The host calls
`main(num_iterations, inner_iterations)`, which returns `true` after every iteration has
verified its result; a failed verification panics.

| Benchmark | File | Uses `som.rn` | Inner iterations (rebench.conf) |
|---|---|---|---|
| Bounce | bounce.rn | yes | 1500 |
| List | list.rn | no | 1500 |
| Mandelbrot | mandelbrot.rn | no | 500 (verified: 1, 500, 750) |
| NBody | nbody.rn | no | 250000 (verified: 1, 250000) |
| Permute | permute.rn | no | 1000 |
| Queens | queens.rn | no | 1000 |
| Sieve | sieve.rn | no | 3000 |
| Storage | storage.rn | yes | 1000 |
| Towers | towers.rn | no | 600 |
| Havlak | havlak.rn | yes | 1500 (verified: 1, 15, 150, 1500, 15000) |
| Json | json.rn | yes | 100 |
| CD | cd.rn | yes | 250 (verified: 2, 10, 100, 200, 250, 500, 1000) |

## Differences from the Lua versions

- Indices are 0-based, with the index arithmetic of the Java versions.
- `nil` is `Option`. `Vector.at` returns `None` past the storage, as Lua returns `nil`.
- Rune has no reference identity, and `==` on structs compares fields recursively. Objects
  that AWFY compares by identity define `same(self, other)` over a unique integer field;
  `Vector.remove`, `IdentitySet` and `IdentityDictionary` use it.
- Rune closures cannot assign captured variables, so such variables are one-element
  vectors, as the Java versions use one-element arrays.
- Rune has no inheritance. Subclasses repeat the inherited methods, or delegate to a free
  function such as `default_inner_benchmark_loop`.
- Rune has no mutable statics. Lua's module-level objects live in fields of the object that
  uses them.
- Rune has no clock in its default modules, so the harness does not time iterations; time
  the process from outside.
