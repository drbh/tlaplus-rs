# Experimental modern specification DSL

`.tsl` is a typed action language that compiles to ordinary TLA+ and uses the
existing checker. Every original `.tla` example has a `.tsl` counterpart beside
it: see the [18-pair comparison index](../README.md#modern-syntax-comparisons).

```sh
cargo build -p tlaplus-cli
target/debug/tla examples/03-invariants/Duplicates.tsl
target/debug/tla examples/08-race/Race.tsl
target/debug/tla --check examples/15-modules/Point.tsl
target/debug/tla --export examples/13-fairness/Fairness.tsl /tmp/tsl-fairness-export
```

Install the [local VS Code extension](../../editors/vscode/README.md) for
highlighting that follows your editor theme.

Export produces a standalone `.tla` module and `.cfg` configuration, including
imported definitions. Existing output files are not overwritten. Generated
operators have source-location comments; the compiler API returns their origins.
Safety traces include action names and source locations. Temporal traces currently
show states and cycle boundaries without source-level action labels.

## Additional small examples

| Example | Expected result | What it tests |
| --- | --- | --- |
| `AtomicCounter.tsl` | Pass; 4 reachable states | Atomic increments |
| `Race.tsl` | `BothIncrementsCounted` fails | Split read/write loses an update |
| `Lock.tsl` | Pass; 3 reachable states | Mutual exclusion |
| `BrokenLock.tsl` | `MutualExclusion` fails | Acquisition ignores the lock |
| `Retry.tsl` | Pass; 3 reachable states | Delivery under strong fairness |
| `UnfairRetry.tsl` | `DeliveredEventually` fails | Weak fairness permits repeated loss |
| `Choice.tsl` | `BelowThree` fails | Every candidate is explored |

These are separate from the ports beside the original lessons. For example,
`09-lock/Lock.tsl` models the original two-thread counter algorithm;
`17-modern-dsl/Lock.tsl` is a smaller mutual-exclusion example.

## State and actions

```rust
spec Counter {
    const Limit: Int = 3;
    state count: Int = 0;
    action increment {
        require count < Limit;
        next count = count + 1;
    }
    action finished { require count == Limit; }
    fair weak increment;
    invariant Bounded { count >= 0 && count <= Limit }
    property Finishes { eventually(count == Limit) }
}
```

By default, every named action participates in the next-state relation. An explicit
`next { send, receive, idle }` selects only those action families; other actions
remain available as helpers. `require false`
disables the action; it does not report a property violation. All guards and
right-hand sides read the old state, and all writes commit simultaneously.
Unwritten state is preserved automatically. Writing a variable twice is an
error. `next x = y; next y = x;` swaps the values.

`pick digit in 0..=9;` introduces a nondeterministic choice, explored exhaustively
during model checking. Picks must precede guards and updates; later domains may
use earlier picks. Empty domains disable the action. Picks are not stored state.

Parameterized actions name a family of transitions:

```rust
action finish(worker in Workers) {
    require !done[worker];
    next done = put(done, worker, true);
}
```

Parameter domains must be constant sets. Every parameter combination is explored.
Domains of local `pick` statements may also read current state.

Initializers are constant expressions. Membership initialization allows multiple
initial states without adding a setup transition:

```rust
state seq: Seq<Int> in sequences(1..=4, 3);
```

This explores all 64 sequences of length three at initialization. Declaration
initializers cannot read other state variables. Use an explicit
`init` block to correlate initial values:

```tsl
state sent: Int;
state received: Int;
init {
    pick seed in 0..=2;
    next sent = seed;
    next received = seed;
}
```

Here `next` binds the initial value, not a primed variable. All state must be
assigned exactly once. Use either declaration initializers or an `init` block,
not a mixture. `require` inside `init` constrains candidate initial values.
Initial expressions cannot read current state or state-reading views.

`call helper(arguments);` composes an action into the same atomic step. All
updates still read the old state. The compiler renames local choices, combines
guards and updates, and adds unchanged-state clauses only after composition.
Duplicate writes, even across helpers, are rejected. Arguments must belong to
the helper's parameter domains for the composed action to be enabled. These
are action-enabling conditions, not function refinement contracts. Calls are
acyclic. Fairness names must be selected by an explicit `next` block.

The [reliable delivery model](../18-reliable-delivery/README.md) exercises these
features with two clients and a lossy network.

## Types and expressions

Types are `Int`, `Bool`, `String`, nominal enums, `Set<T>`, `Seq<T>`, and `Map<K,V>`.
An enum such as `enum Phase { Ready, Busy, Done }` also introduces a set `Phase`
containing its variants. Variants are globally unique, unqualified names and
are encoded as tagged strings. Maps are finite partial functions; require an
exact domain separately when that is part of an invariant.

Sequences use one-based indexing, matching TLA+. `[1, 2, 3]` is a sequence;
`{1, 2, 3}` is a set; `(1, true)` is a heterogeneous tuple. Tuple expressions are
available for facts and choices; heterogeneous tuple indexing requires a literal
index. Collections can nest. Empty collections acquire element types from their
context. Strings support escaped quotes, backslashes, newlines, tabs, and carriage
returns; multiline string literals are not supported.

Expressions support arithmetic `+ - * %`, comparisons `== != < <= > >=`, Boolean
`! && ||`, implication `=>`, and inclusive ranges `lo..=hi`. Precedence is unary,
multiplication/modulo, addition, range, comparison, `&&`, `||`, then implication.
Implication associates to the right. Use parentheses instead of chained
comparisons. `if condition { value } else { value }` is an expression, and its
branches must have compatible types.

| Operation | Syntax |
| --- | --- |
| Sequence operations | `len(s)`, `head(s)`, `tail(s)`, `append(s, x)`, `concat(a, b)`, `slice(s, lo, hi)` |
| Set operations | `size(s)`, `contains(s, x)`, `union(a, b)`, `intersect(a, b)`, `difference(a, b)`, `subset(a, b)`, `powerset(s)` |
| Map operations | `m[key]`, `domain(m)`, `put(m, key, value)`, `entry(key, value)`, `merge(a, b)` |
| Finite domains | `sequences(elements, length)`, `sequences_up_to(elements, max_length)`, `functions(keys, values)`, `product(a, b)`, `product3(a, b, c)` |
| Quantifiers | `forall(x in domain) { predicate }`, `exists(x in domain) { predicate }` |
| Map construction | `map(x in domain) { value }` |
| Sequence construction | `sequence(i in 1..=length) { value }` |
| Set image/filter | `set_of(x in domain) { value }`, `filter(x in domain) { predicate }` |
| Mathematical choice | `choose(x in domain) { predicate }` |

`choose` maps to TLA+ `CHOOSE`, a deterministic mathematical choice, unlike
nondeterministic `pick`. `put` preserves other entries and does not extend a map's
domain. `merge` follows TLC's left-biased function override. Collection operations
report invalid accesses at evaluation time; static types do not prove bounds.

Nominal records describe messages with named fields:

```tsl
struct Message { peer: Peer, kind: MessageKind }
state pending: Set<Message> = {};
// Construct: Message { peer: Alice, kind: Request }
// Read: message.peer, message.kind
```

Constructors must supply every field exactly once with the declared type.
Two differently named structs remain different types even with identical
fields. Structs are nonrecursive; fields use base types, enums, or earlier
structs. Put value constraints in a refinement of the struct. Payload-carrying
enums and pattern matching remain future work.

## Functions, assumptions, and modules

```rust
fn absolute(n: Int) -> Int { if n < 0 { -n } else { n } }
recursive fn factorial(n: Int) -> Int {
    if n == 0 { 1 } else { n * factorial(n - 1) }
}
assume factorial(5) == 120;
```

Functions are pure, with explicit parameter and result types. They can read
constants and call other pure functions. Definitions are ordered by dependency;
forward references work, but mutual cycles are rejected. Self-recursion requires `recursive fn`;
termination is the author's responsibility. `assume` checks a constant Boolean
fact before exploration. Constants may call pure functions as well as builtins. A cycle between a
constant, function, or refinement predicate is rejected.

`view acknowledged(p: Peer) -> Bool { completed[p] }` defines a reusable
state-reading expression. Views can call earlier or later views and pure
functions; pure functions and initializers cannot depend on views. Views may
return values as well as Boolean predicates, making them useful for abstract
state projections. `after(view_name())` is allowed in transition properties.

Higher-order parameters use `Fn<Input, Output>`. For example:

```rust
fn seq_map(op: Fn<Int, Int>, values: Seq<Int>) -> Seq<Int> {
    sequence(i in 1..=len(values)) { op(values[i]) }
}
assume seq_map(lambda(n: Int) { n + 1 }, [1, 2]) == [2, 3];
```

Lambdas are pure and take one explicitly typed value argument. Operators cannot
be stored in state or collections, returned from functions, or passed to a
recursive function. Refined lambda parameters are checked when evaluated. Refinements inside
`Fn` signatures are not supported; use a value-taking wrapper. There is no
polymorphic function inference yet.

`module Point { ... }` defines a library of enums, structs, refinements,
exploration domains, constants, pure functions, and assumptions. `import Point;` loads sibling `Point.tsl`. Imports share one namespace;
duplicate definitions and cycles are errors, and repeated imports are deduplicated.
Use explicit function parameters in place of TLA+ module instantiation. Libraries
are checked with `--check`, not run as models. `compile_file(path)` resolves
imports; `compile(source)` handles self-contained source strings.

## Properties and fairness

- `invariant Name { predicate }` checks every reachable state. State types are
  also exported as a `TypeOK` invariant, never used as a state-space filter.
- `property Name { formula }` checks a temporal formula built from `always`,
  `eventually`, Boolean operators, and quantifiers over constant domains.
  `always(requested => eventually(delivered))` is a response property.
- `transition Monotonic over (counter) { after(counter) >= counter }` checks a
  pair of states. The frame explicitly names the variables permitted to stutter,
  preserving TLA+'s `[][A]_v` semantics. `after` is forbidden in guards, updates,
  and state invariants; action formulas cannot be mixed directly into temporal
  properties.
- `fair weak finish;` requires a continuously enabled action to eventually occur.
  `fair strong finish;` requires an infinitely often enabled action to occur
  infinitely often. For a parameterized action, fairness applies separately to
  each parameter combination. For a local `pick`, fairness applies to the whole
  action, not separately to each picked value.
- `fair weak { get_lock, get_counter, inc_counter, release_lock };` applies one
  fairness assumption to the union of these actions, separately for each shared
  parameter combination. Group members must have identical parameter names and
  domains. This preserves the original Fairness lesson's `WF_vars(Thread(t))`.

The export uses `Spec == Init /\ [][Next]_vars` plus explicit fairness assumptions.
There is no default fairness. Stuttering remains part of temporal semantics.
Deadlock detection is enabled independently of stuttering; a terminal model should
provide a guarded self-loop, as `finished` does. `--no-deadlock` disables that
check when stopped states are intentionally acceptable.

## Refinement predicates and exploration domains

[RiverCrossing.tsl](../16-puzzles/RiverCrossing.tsl) gives the boat and the river
state their own refinement types:

```tsl
enum Character { Farmer, Wolf, Goat, Cabbage }
enum Bank { Left, Right }

type Crossing = Set<Character> where crew {
    contains(crew, Farmer) && size(crew) <= 2
}
type SafeRiver = Map<Character, Bank> where banks {
    banks[Farmer] == banks[Goat]
    || (banks[Wolf] != banks[Goat] && banks[Goat] != banks[Cabbage])
}

state side: SafeRiver = map(c in Character) { Left };

action cross(crew in Crossing) {
    require forall(c in crew) { side[c] == side[Farmer] };
    require crossed(side, crew) is SafeRiver;
    next side = crossed(side, crew);
}
```

The complete example defines `crossed` as a pure function that flips the bank
of each crew member. A crossing is a set containing the farmer and zero or one
passengers. The type therefore excludes a missing farmer, duplicate passengers,
and overloaded boats. The `SafeRiver` base is the set of **total** maps: every
character has exactly one bank. Its predicate excludes unsafe arrangements.
The original separate `AllItems` invariant is no longer necessary.

A refinement declaration defines a **predicate independently of enumeration**.
`value is SafeRiver` checks that predicate. A state annotation adds it to the
generated `TypeOK` invariant. Invalid initializers or updates are violations,
not implicit guards; removing the example's safety guard exposes a `TypeOK`
counterexample. The guarded model still has a seven-crossing solution and ten
reachable states when the deliberate `NotSolved` failure is disabled.

Finite exploration domains are declared separately:

```tsl
type Percent = Int where n { n >= 0 && n <= 100 }
domain Percent in 0..=2;
fn increment(n: Percent) -> Percent { n + 1 }
state percent: Percent in Percent;
```

The domain is filtered by the predicate and supplies values for `in Percent`
and `contains(Percent, value)`. It does not narrow the type: `50 is Percent`
is true. State can leave the exploration domain while remaining within its
type. A fixed initializer can use `50` without declaring a domain at all.
Domains bound the choices explored by this model, not every reachable state.
Choose them deliberately; checking a small domain does not establish behavior
for every possible input.

For compatibility, refinements of enumerable bases (`Bool`, enums, earlier
finite domains, sets, and maps) receive a default domain. `Int`, `String`,
sequences, and records can be refined without one; provide an explicit domain
when using them as choices. Domains may call pure functions and use constants.
Do not assume a finite candidate expression is cheap to enumerate.

Predicates can call pure functions and refer to constants. Nested annotations
such as `Seq<Percent>` and `Map<Peer, Count>` are checked recursively. Refined
function parameters and results produce named precondition/postcondition
assertions at evaluated calls; invalid calls report an assertion error rather
than silently disabling an action. Refined constants generate checked
`ASSUME` obligations. These checks do not prove contracts for uncalled inputs,
and `--check` validates static types, not the truth of refinement predicates.

Maps **inside a refinement's base type** are total mathematical functions,
including nested maps. Their domains are determined by their key types, not by
configured exploration bounds. Ordinary `Map` annotations continue to permit
partial maps. Use finite key types for total maps explored by the checker.

This is model-checked refinement validation, not SMT-backed compile-time
proofs. Contracts are pure predicates and cannot depend on current state.

## Validation and limits

Native regression tests compare every port with its original. The original
Dedup enables symmetry reduction; its modern port uses enum values and explores
all inputs without reduction. The regression test also compares the original
with symmetry disabled. Other passing ports match original state counts.

TLC 1.8.0 independently checks the exported ports and the seven small models.
Repeat that comparison with:

```sh
JAVA=/absolute/path/to/java \
TLA2TOOLS_JAR=/absolute/path/to/tla2tools.jar \
cargo test -p tlaplus-cli --test tlc_dsl -- --ignored
```

The tests compare verdicts and complete successful state counts. Partial counts
at the first failure can differ with exploration order. These finite comparisons
are not a proof of compiler correctness or a usability study.

Types do not make a model finite. Use bounded domains and guards, and optionally
`--max-states N`. Exit codes are 0 for success, 1 for a violation or deadlock, 2
for a compilation/runtime error, and 3 for a state-limit stop. The backend uses
signed 32-bit integers and reports overflow. Expressions are limited to 256
syntax nodes and 128 nesting levels; types to 32 nesting levels.

The language still has no payload-carrying enums, pattern matching, polymorphic
functions, namespaced or parameterized module instances, process notation,
symmetry declarations, SMT refinement proofs, formatter, or LSP. Unsupported syntax is rejected.
`--config` cannot override a `.tsl` model; export first to experiment with TLC
configuration. A comparison of the notation's usability against PlusCal and
Quint remains future work.
