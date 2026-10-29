# Puzzle gallery

Nine small models demonstrate Boolean reasoning, arithmetic, spatial planning,
and practical scheduling. Open them in the playground's example selector, or run:

```sh
cargo run -p tlaplus-cli -- examples/19-puzzle-gallery/BridgeAndTorch.tsl
```

Each default model deliberately violates `NoSolution` or `NotSolved` (exit code
1). Its counterexample is the answer. This is an existential search: finding one
successful route does not mean every route succeeds. Each default model finishes
within the playground's 500-state limit. The TSL models can be exported with
`tla --export FILE.tsl DIRECTORY`; the playground also lets you open the generated
TLA+ and configuration as an editable native model.

| Model | Rules and purpose | Expected result |
| --- | --- | --- |
| [KnightsAndKnaves](KnightsAndKnaves.tsl) | Three people make truth/lie statements; truthfulness must agree with each statement. | One assignment: A and C are knaves; B is a knight. |
| [FeatureSelection](FeatureSelection.tsl) | At least three features, five-credit budget, dependencies, and an offline/sync conflict. | Three valid packages; the default run reveals one. |
| [MiniEinstein](MiniEinstein.tsl) | Three houses, jobs, and pets; distinctness and adjacency clues. | One assignment: doctor/dog, baker/cat, teacher/fish, left to right. |
| [Cryptarithm](Cryptarithm.tsl) | `I + BB = ILL`, distinct decimal digits, no leading zeroes. | One answer: `1 + 99 = 100`. |
| [MiniSudoku](MiniSudoku.tsl) | Complete a 4×4 grid, preserving clues and distinct rows, columns, and 2×2 boxes. | Seven placements; a unique completed grid. |
| [BridgeAndTorch](BridgeAndTorch.tsl) | Crossing times 1/2/5/10, one torch, one or two people per crossing. | Five crossings totaling 17 minutes. Budget 16 has no solution. |
| [WarehouseRobot](WarehouseRobot.tsl) | Walk or push, never pull, on a 4×4 floor with a wall at (2,3). | Four moves deliver the crate from (2,2) to (3,3). |
| [StaffRota](StaffRota.tsl) | Three staff cover six shifts, each works twice; availability and overnight rest apply. | A valid six-slot schedule, e.g. Alice/Bob/Cara/Bob/Cara/Alice. |
| [MiniFactory](MiniFactory.tsl) | Three two-operation jobs share drill/paint machines; enforce precedence and nonoverlap. | Starts `[0,0,2,2,3,3]` meet deadline 5. Deadline 4 is infeasible. |

## Read the models correctly

The first four are static constraint problems. `init` chooses candidate values
and `require` keeps consistent assignments. The solution is an initial state,
not a sequence of logical deductions. The engine checks invariants before
inserting a state into its visited table, so an initial-state solution can show
zero retained states alongside a one-state counterexample. That is not an empty
solution. The playground provides assignment views alongside the generic values.

The remaining models construct plans or assignments through actions. Sudoku,
staff rota, and factory scheduling use a fixed decision order to avoid exploring
all permutations of equivalent decisions. Their steps are placements/assignments,
not simulated elapsed time. In particular, the factory's six action steps do not
mean six minutes: `starts` records scheduled times and the source defines durations.

`inspect` is an explicit self-loop. It lets a rejected partial assignment or
exhausted budget stop without reporting a system deadlock. It does not create a
solution. This gallery is about existence of solutions; it does not claim eventual
success under arbitrary choices, and does not add fairness assumptions.

The bridge explicitly records elapsed time and bounds it by `Budget`, keeping
the search finite. Breadth-first search minimizes action count, not elapsed cost.
A solution at 17 and exhaustive absence of one at 16 establish the minimum time
for these integer crossing costs. Similarly, factory feasibility at 5 and
infeasibility at 4 establish its minimum integer makespan. Exhausting a resource
limit would establish neither result.

## Experiments

- **Uniqueness:** remove only the `NoSolution` invariant in a static model and
  rerun. All consistent initial assignments are then explored: one each for the
  knights, houses, and cryptarithm; three for feature selection. Do not replace
  it with a constant-false invariant: the checker rejects that configuration.
- **Missing clue:** remove the cat/doctor adjacency requirement in MiniEinstein.
  More assignments become possible; find how many.
- **Feature budget:** change `Budget` to 4. Which package disappears? Try forcing
  both Offline and CloudSync to observe an inconsistent specification.
- **Sudoku:** change a clue or remove one. `NoConflicts` detects contradictory
  filled cells; all successful solutions preserve the remaining clues.
- **Planning bound:** lower bridge `Budget` to 16 or factory `Deadline` to 4. Both
  complete exploration with the no-solution property holding, within 500 states.
- **Irreversibility:** change the robot/crate initial positions. Which pushes
  make the goal unreachable? With `NotSolved` removed, the default model explores
  210 legal robot/crate configurations.
- **Scheduling:** change availability, workloads, machine assignments, or operation
  durations and observe how feasibility changes.

The playground's puzzle views project the selected state's actual data. They do
not run another solver. A mismatched state schema falls back to the generic
inspector. The warehouse view shows positions only: walls/goal remain explicit
in the source. The factory view shows start times, not a Gantt chart with assumed
hard-coded durations. Full domain diagrams need configuration metadata as well
as state, especially when users edit capacities or durations.

## Validation and background

Native tests check the answers independently, count consistent assignments,
validate the completed Sudoku and schedules, and check the tighter infeasible
budgets. Optional TLC tests check all nine exports plus the two infeasible cases:

```sh
cargo test -p tlaplus-playground --test gallery
JAVA=/path/to/java TLA2TOOLS_JAR=/path/to/tla2tools.jar \
  cargo test -p tlaplus-playground --test gallery -- --ignored
```

The small story instances and models here are written for this repository.
Cryptarithmetic, staff scheduling, and job-shop scheduling have useful background
in the official OR-Tools guides:
[cryptarithmetic](https://developers.google.com/optimization/cp/cryptarithmetic),
[employee scheduling](https://developers.google.com/optimization/scheduling/employee_scheduling),
[job shop](https://developers.google.com/optimization/scheduling/job_shop).
These explicit-state models are educational finite instances, not replacements
for an industrial SAT/SMT or constraint-programming solver.
