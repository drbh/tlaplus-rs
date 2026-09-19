# examples

a numbered path through TLA+ with `tla`, loosely following the core track of
https://learntla.com but in plain TLA+ rather than PlusCal. each lesson is one
spec with a comment at the top saying what it teaches, how to run it and what
to expect. Every `.tla` source has a `.tsl` counterpart beside it. TLA+ config
lives in a `CONFIG` section; modern models declare constants and properties
directly in the source.

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

## Modern syntax comparisons

Open each pair side by side, or run it by changing only the file extension:

```sh
./target/release/tla examples/03-invariants/Duplicates.tla
./target/release/tla examples/03-invariants/Duplicates.tsl
```

| Original | Modern port |
| --- | --- |
| [Counter.tla](01-counter/Counter.tla) | [Counter.tsl](01-counter/Counter.tsl) |
| [Operators.tla](02-operators/Operators.tla) | [Operators.tsl](02-operators/Operators.tsl) |
| [Duplicates.tla](03-invariants/Duplicates.tla) | [Duplicates.tsl](03-invariants/Duplicates.tsl) |
| [DuplicatesBug.tla](04-counterexample/DuplicatesBug.tla) | [DuplicatesBug.tsl](04-counterexample/DuplicatesBug.tsl) |
| [Dedup.tla](05-constants/Dedup.tla) | [Dedup.tsl](05-constants/Dedup.tsl) |
| [Bank.tla](06-functions/Bank.tla) | [Bank.tsl](06-functions/Bank.tsl) |
| [Calculator.tla](07-nondeterminism/Calculator.tla) | [Calculator.tsl](07-nondeterminism/Calculator.tsl) |
| [Race.tla](08-race/Race.tla) | [Race.tsl](08-race/Race.tsl) |
| [Lock.tla](09-lock/Lock.tla) | [Lock.tsl](09-lock/Lock.tsl) |
| [Deadlock.tla](10-deadlock/Deadlock.tla) | [Deadlock.tsl](10-deadlock/Deadlock.tsl) |
| [ActionProperties.tla](11-action-properties/ActionProperties.tla) | [ActionProperties.tsl](11-action-properties/ActionProperties.tsl) |
| [Liveness.tla](12-liveness/Liveness.tla) | [Liveness.tsl](12-liveness/Liveness.tsl) |
| [Fairness.tla](13-fairness/Fairness.tla) | [Fairness.tsl](13-fairness/Fairness.tsl) |
| [Advanced.tla](14-advanced-operators/Advanced.tla) | [Advanced.tsl](14-advanced-operators/Advanced.tsl) |
| [Modules.tla](15-modules/Modules.tla) | [Modules.tsl](15-modules/Modules.tsl) |
| [Point.tla](15-modules/Point.tla) | [Point.tsl](15-modules/Point.tsl) |
| [RiverCrossing.tla](16-puzzles/RiverCrossing.tla) | [RiverCrossing.tsl](16-puzzles/RiverCrossing.tsl) |
| [RiverCrossingSimple.tla](16-puzzles/RiverCrossingSimple.tla) | [RiverCrossingSimple.tsl](16-puzzles/RiverCrossingSimple.tsl) |
| [DieHard.tla](16-puzzles/DieHard.tla) | [DieHard.tsl](16-puzzles/DieHard.tsl) |

`Point.tsl` is an imported library, not a runnable state machine:

```sh
./target/release/tla --check examples/15-modules/Point.tsl
./target/release/tla examples/15-modules/Modules.tsl
```

The ports preserve the configured input domains, atomic boundaries, and expected
verdicts. Passing models have matching state counts except Dedup, whose modern
port deliberately runs without symmetry reduction. With symmetry disabled in
its original too, both explore 666 states from 120 initial inputs.

Representation differences are documented at the top of each port:

- Enums replace program-counter strings and opaque Dedup elements. The lock
  examples use 0 for NULL; thread IDs remain 1 through NumThreads.
- Bank retains its balance map and groups the last transfer in a typed record.
  Deadlock uses typed locks, maps, and parameterized acquire/release actions.
- Modules imports a Coordinates struct and point functions; typed Origin and
  Unit values replace parameterized TLA+ module instances.
- Advanced uses named functions instead of a custom infix operator, nested `if`
  instead of `CASE`, and a recursive function declaration for factorial.
- RiverCrossing uses bounded `Crossing` and `SafeRiver` refinement types to
  describe legal boat crews and safe, total bank assignments. RiverCrossingSimple
  retains Booleans for banks as a direct comparison.

All runnable models select their root actions explicitly with `next { ... }`.
Calculator, AtomicCounter, and the retry examples compose shared helpers inside
one atomic step; helpers cannot run independently. State views name reusable
queries in the duplicate, concurrency, bank, and puzzle examples. Choice separates
its refinement predicate from its finite exploration domain, and factorial uses
refined argument and result contracts. The typed river accepts a safe input and
a legal crew, then checks the candidate result before committing it.

The simpler sequence and Boolean representations remain where they teach the
original lesson. Intentional failures retain their original properties and
fairness assumptions.

The comparisons exercise finite instances, not a proof of compiler equivalence.
See the [modern language guide](17-modern-dsl/README.md) and
[VS Code extension](../editors/vscode/README.md) for syntax and editing support.


## Composable protocol example

[18-reliable-delivery](18-reliable-delivery/README.md) models two clients over a
lossy network with retries and duplicate deliveries. It uses nominal messages,
explicit initialization and action selection, atomic helpers, state views, and
refined function contracts. The native checker and TLC both explore 100 states
and verify the configured safety and eventual-completion properties.

## Die Hard water jugs

[DieHard.tsl](16-puzzles/DieHard.tsl) models unmarked three- and five-gallon
jugs. Fill either jug, empty it, or pour until the source is empty or the
receiving jug is full. The goal is exactly four gallons in the large jug.
A `Jug` enum identifies the containers; the `Volumes` refinement requires a
complete map whose amounts stay within each jug's capacity.

```sh
./target/debug/tla examples/16-puzzles/DieHard.tsl
```

`NotSolved` deliberately fails (exit code 1), revealing a shortest six-move
solution. In `(small, large)` gallons:

`(0, 0) → (0, 5) → (3, 2) → (0, 2) → (2, 0) → (2, 5) → (3, 4)`

Change `invariant NotSolved { !solved() }` to `invariant NotSolved { true }`
to check the volume bounds over all 16 reachable states. The paired
[DieHard.tla](16-puzzles/DieHard.tla) expresses the same moves directly in TLA+.

[DieHardTyped.tsl](16-puzzles/DieHardTyped.tsl) is a second implementation with
separate `SmallJug` and `LargeJug` structs. The compiler rejects assigning one
jug type to the other. `ValidSmallJug` and `ValidLargeJug` add the respective
0–3 and 0–5 gallon bounds, checked by the model checker on initial and reachable
states. Six named actions replace the map version's parameterized actions.
Both versions find a six-move solution and explore 16 states with `NotSolved`
disabled; tests also check that overflowing either jug violates its refinement.

```sh
./target/debug/tla examples/16-puzzles/DieHardTyped.tsl
```
