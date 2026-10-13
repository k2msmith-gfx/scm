# The R7RS benchmark suite, two ways

[ecraven/r7rs-benchmarks](https://github.com/ecraven/r7rs-benchmarks): 57
programs, the Gabriel and Gambit benchmarks revised for R7RS. Cloned rather
than vendored -- 6 MB of inputs and mixed copyright headers with no licence
file:

    scm-conform/bench/run.py --fetch          # clones into target/r7rs-benchmarks

**As a language gate**, `cargo test -p scm-conform --test benchmarks`:
`inputs-small/` holds one input per program with the iteration count at 1
and the problem size cut until T0 finishes in seconds, plus the answer it
must return. Every program runs and is right; from S12 on, every tier agrees.
The answers were taken from T0's own first run and spot-checked by hand --
the gate's value is agreement and regression, not independent truth.

**As a performance instrument**, by hand, never in CI:

    scm-conform/bench/run.py [--small] [timeout] [names...]

With the published inputs this is hours at T0's speed; the comparison is the
suite's own `results.*` files. See docs/conformance.md for the numbers.

## Outside the declared subset

| program | needs | why not |
|---|---|---|
| `pi`, `chudnovsky` | bignums | plan §2.3: the numeric tower is not implemented |
| `compiler` | a literal `9223372036854775808` | the same -- the program is otherwise fine |
| `mbrotZ` | complex numbers `-1.0-0.5i` | the same |

The other 53 are in the gate.
