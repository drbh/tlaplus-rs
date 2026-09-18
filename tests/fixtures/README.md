`Counter.tla` is a small rewrite-owned model used with several test configurations.
`Choices.tla` exercises multiple initial states, split/duplicate actions, config
overrides, constraints, VIEW, and model-value symmetry in scoped evaluator tests.

`HourClock.tla` and the effective statements in `HourClock.cfg` come from the
Examples corpus, `specifications/SpecifyingSystems/HourClock/`. They allow the
faithful-port smoke test to compare with the existing Java recording without
requiring a local Examples checkout or a JVM. The recording remains in the shared
`tests/tlc-oracle/results/examples/` directory at the repository root.

`Level.tla` checks `TLCGet("level")` during initial generation, successor generation
and invariant evaluation. Its complete graph has four states at depths 1–4.

`AnyMembership.tla` checks universal-set membership, finiteness and rendering.
`AnyShadow.tla` ensures state-dependent user definitions named `Any` and
`JavaTime`, and a user-defined `TLCModelValue`, are preserved.

`ModelValues.tla` checks dynamically created typed model values against a configured
model value. Two initial states generate fresh labels on worker threads.
`OperatorShadow.tla` checks user-defined arithmetic, sequence and named operators.
`SubsetAssignment.tla` checks subset assignment in initialization, successors and
ENABLED. `UniversalAssignment.tla` covers generating universal quantifiers, including
contradictory and empty domains and a 1,000-element domain.
`RecursiveAssignment.tla` passes unassigned variables through recursive operators.
`EnabledGuard.tla` checks disabled-action short-circuiting and non-stuttering actions.

`CaseActions.tla` checks assignment generation through CASE and lazy branch selection.
`ActionFunctions.tla` checks function-valued actions, conditional function selection,
UNCHANGED, infinite domains and lazy function updates used as guards. Its alternative
configurations verify that invalid domains and function-driven initialization are
rejected by both pipelines.

`EnabledInstances.tla` and `EnabledTemplate.tla` check variable substitution scopes
around ENABLED, captures, UNCHANGED and recursive/nested checks.
`EnabledNested.tla`, `EnabledOuter.tla` and `EnabledLeaf.tla` check that an inner
INSTANCE preserves both interpretations of its outer variable bindings. Both
pipelines explore the same two states, including separate-process successor replay.

`RecursiveProjection.tla` checks nested recursive-function applications and DOMAIN
without materializing unselected entries. It covers tuple arguments and captured
variables. `StateOperators.tla` passes prime, UNCHANGED and ENABLED as operator
arguments; ENABLED guards are checked in NEXT because the faithful front end has a
known level-checking bug when these higher-order expressions are invariants.

`Fingerprints.tla` checks native TLCFP on updated functions and supplies expression
comparisons for scalar, Unicode, model, set and function values under multiple
fingerprint polynomials.

`PowerSetInclusion.tla` checks symbolic inclusion of power sets whose expansion
would exceed the enumeration limit. `ConfiguredOperators.tla` checks unary and
binary configuration tables, higher-order uses, undefined table entries and a
configured TLCEval override. `RecursiveOperators.tla` checks recursive operators
with distinct built-in/lambda arguments and captured state/local values.

`AssertionFailures.tla` and its `Assertion*.cfg` files exercise assertion failures
in initialization, successor generation, invariants, state constraints and action
properties. An unclassified arithmetic type error must not pass the rejection checker.
`RejectionDomains.tla` checks empty Head/Tail, CHOOSE without a witness and CASE
without a true branch at different exploration phases. It also checks non-enumerable
quantifier domains, division by zero and non-positive modulus operands.
`UnexpandedSnapshot.tla` checks that snapshot collection does not evaluate queued
states after an initial invariant violation has stopped exploration.

`ChooseUnion.tla` checks canonical CHOOSE order over a union whose left-hand set
starts with the larger value; the runtime enumeration order must not change the
selected state value.

`AssertErrors.tla` checks native TLCExt AssertError, including configured aliases,
exact supported error text, rejected non-literal messages, higher-order calls,
bound variables and primed expressions. Unknown error text stays explicitly
unsupported.

`RandomSampling.tla` checks exact seeded RandomElement/RandomSubset streams over intervals,
normalized sets, large products, functions and records, nested draws, and draw
positions after errors. Sampling definitions take an unused parameter to prevent
TLC's setup constant pre-evaluation from consuming or caching the test draws.
It includes zero-sized and oversized subset requests, large product indices and
a function space with 2^150 members. These are expression-session tests; they do
not establish model-execution replay.

`RandomModel.tla` and its configurations check random generation interleaved with
invariant/action-constraint draws, setup ASSUMEs, canonical fingerprints under VIEW
and SYMMETRY, and random invariant counterexamples. The conformance tests compare
serial/parallel exploration and validate the witnesses using bounded execution
replay when callbacks draw randomness.

`IncompleteStates.tla` checks that initial and successor generation report every
unassigned variable by name. Declarations are deliberately out of alphabetical
order. The rejection gate compares missing names, phase and current state across
the faithful adapter and native one-/four-worker runs.

`InfiniteOperators.tla` checks deferred power sets and printing, direct cardinality
errors for Nat/Int/STRING, operand-before-rendering error order, mixed-set rendering
errors, and normalized set text. Cardinality accepts enumerable sets and rejects
scalars, functions, tuples, records and sequence sets (including Seq({})). Its
configurations exercise infinite-cardinality failures in invariants and VIEW
canonicalization.

`InvalidSelection.tla` checks out-of-bounds tuple indexes, missing record fields
and application of intervals. The rejection gate compares the offending index or
field and rendered value as well as the phase and state.

`SequenceBounds.tla` checks SubSeq's empty result for reversed bounds, including
out-of-domain and extreme integer bounds, on tuples and strings. Paired tests
compare bound/type diagnostics and which invalid argument fails first. Its
configurations exercise invalid bounds, Len/Append operand types and Tail on an
empty string through invariant checks.

`RuntimeRegisters.tla` exercises indexed and named register writes during initial
and successor generation, reads from invariants, and replay of an invariant
counterexample. Native serial execution is checked against the faithful driver,
including repeated runs and fresh clones. `RuntimeReads.tla` checks precise
undefined-register diagnostics through the rejection pipeline. These fixtures do
not establish setup ASSUME register snapshots or parallel register callbacks.

`WorkerRegisters.tla` checks worker-local register values and writes from an
invariant that affect the next generated successor. It matches the faithful
four-worker driver's successful state count/depth and the native complete state
set with one, two and four workers. Engine tests additionally prove overlapping
generation, callback affinity, and termination without retries after failures.
Parallel register-dependent counterexample schedule replay remains separate work.

`AssumedRegisters.tla` writes indexed and named registers in ASSUMEs before Init
reads them. Native setup snapshots survive repeated runs, clones and changes in
worker count. Full faithful-driver runs with one and four workers agree on the
three-state successful exploration. The native CLI also tests stopping at the
first false ASSUME before a later undefined register read.

`AllRegisters.tla` checks the indexed and named all-worker views, sparse register
domains, worker tuple lengths and initialization broadcasts. Native and full
faithful-driver runs agree with one and four workers. Runtime unit tests separately
check exact tuples, string ordinals, keys found only on other workers, and missing
worker values. These tests do not establish full checker metadata or general
parallel counterexample replay.

`RandomModel.tla` also checks native ASSUME execution from the faithful pre-ASSUME
stream. Four seeds produce the same initial states as completed faithful setup;
repeated setup, initialization and clones preserve the result. This verifies random
stream continuity, not independent native constant pre-evaluation/caching parity.

`RandomConstants.tla` checks a random configuration override, a random ASSUME,
Init and repeated override reads in an invariant. With seed 1, Init gets the third
draw and the two override reads get the fourth and fifth draws. Both native CLI
and full faithful-driver tests pass with one and four workers. This exposes the
difference between an operator override and a frozen configuration value.

`CachedConstants.tla` checks source-ordered constant-definition evaluation, reuse
of a dependent definition's cached value, and distinct behavior for direct
definition references and configured override reads. A failed unused definition
consumes a random draw before division by zero; later ASSUME, Init and invariant
draws verify that the failed attempt does not rewind the stream. Native CLI and
full faithful-driver runs agree with one and four workers.

`ConstantDependencies.tla` and `ForwardConstantDependencies.tla` check an effectful
override used by another override, cached direct definition references and later
state evaluation. Configuration entries and declarations use different orders.
Native CLI and full faithful-driver tests agree with one and four workers. These
fixtures do not establish arbitrary SANY context-enumeration ordering.

`IntegerComparisons.tla` checks all four `Integers` comparisons with invalid first
and second operands, plus valid boundary relationships. The expression conformance
test compares complete errors against the faithful evaluator. Numeric comparisons
must reject non-integers even when the general value ordering could compare them.
`NaturalComparisons.tla` checks the distinct `Naturals` diagnostic for `>=`,
including aliases and higher-order operator arguments. Faithful lowering uses the
installed implementation to distinguish it from the `Integers` override.

`FunctionEnumeration.tla` compares errors for non-enumerable function domains and
ranges, including sequence bounds and nested function sets. It also checks that
empty domains produce the empty function and empty ranges produce no functions
when the domain is nonempty, without forcing the unused infinite bound.
The same fixture covers record-set field errors and Cartesian-product component
errors, first-seen field order, sequence bounds and empty-product behavior.

`CacheRegisters.tla` counts evaluations of two constant-context `TLCCache` sites.
It checks that setup entries survive initialization, repeated setup and faithful
inspection before lowering, and that native runs and clones with one/four workers
retain the expected count. Runtime tests separately cover misses, distinct keys
and sites, failed-computation retries, and independent cloned cache contexts.

`AssumedRegisters.tla` also covers the register-aware faithful bridge: repeated
faithful exploration restores the setup seed, and lowering after exploration still
seeds native initialization from ASSUMEs rather than the last execution values.
The end-to-end pipeline fixture compares safety answers, successor sets and native
one/four-worker runs. Standalone expression sessions retain their earlier context.

`GuardedAction.tla` preserves guard order across a nested action, preventing
`Head` from running on an empty queue. `RecursiveOperators.tla` also checks
primed recursive calls, higher-order arguments, and captured state reads.

`InvalidFunction.cfg` checks function-domain diagnostics and preserves lazy
function bodies when `EXCEPT` updates an absent key.

The integer/natural cast configurations verify direct and higher-order Java
method provenance. Compound-value cast fixtures remain negative controls for
unclassified diagnostics.

`BooleanContexts.tla` checks source-aware IF and negation diagnostics for strings
and model values, plus IF conditions evaluated through set membership and rendering.
It also distinguishes initial-state, successor and ENABLED guard diagnostics from
ordinary IF evaluation when selecting an action-valued function.
