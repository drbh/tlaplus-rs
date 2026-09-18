# examples

a numbered path through TLA+ with `tla`, loosely following the core track of
https://learntla.com but in plain TLA+ rather than PlusCal. each lesson is one
spec with a comment at the top saying what it teaches, how to run it and what
to expect. the config lives in a `CONFIG` section at the bottom of the file

```bash
cargo build --release -p tla-cli
./target/release/tla examples/01-counter/Counter.tla
```

| | lesson | teaches | result |
| --- | --- | --- | --- |
| 01 | `Counter.tla` | VARIABLE, Init, Next, primes, INVARIANT | passes |
| 02 | `Operators.tla` | operators, IF, booleans, sequences, sets, quantifiers, CHOOSE, LET | passes |
| 03 | `Duplicates.tla` | an algorithm as a state machine, pc, type invariants, `=>` | passes |
| 04 | `DuplicatesBug.tla` | reading a counterexample trace | violation |
| 05 | `Dedup.tla` | CONSTANT, ASSUME, model values, SYMMETRY, state sweeping | passes |
| 06 | `Bank.tla` | functions, EXCEPT and `@`, records, function sets | passes |
| 07 | `Calculator.tla` | nondeterminism with `\E` and `\/`, asking questions with invariants | violation |
| 08 | `Race.tla` | threads as functions over a set, interleavings, a race condition | violation |
| 09 | `Lock.tla` | guards, waiting, a NULL model value | passes |
| 10 | `Deadlock.tla` | deadlock, `--no-deadlock`, `:>` and `@@` | deadlock |
| 11 | `ActionProperties.tla` | `[][A]_v`, stuttering, Spec and SPECIFICATION | passes |
| 12 | `Liveness.tla` | `[]`, `<>`, `<>[]`, `~>`, why liveness fails without fairness | violation |
| 13 | `Fairness.tla` | WF and SF, `~>` | passes |
| 14 | `Advanced.tla` | RECURSIVE, higher-order operators, LAMBDA, CASE, recursive functions | passes |
| 15 | `Modules.tla` | INSTANCE, WITH, `!` | passes |
| 16 | `RiverCrossing.tla` | a puzzle solved by asking the checker to disprove it | violation |

the violations are on purpose, the trace is the lesson. `tla` exits 0 when
everything passes and 1 on a violation or deadlock, so this checks all of them:

```bash
for f in examples/*/*.tla; do ./target/release/tla $f > /dev/null 2>&1; echo "$? $f"; done
```
