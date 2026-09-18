---- MODULE InfiniteOperators ----
EXTENDS Integers, Sequences, FiniteSets, TLC
VARIABLE x
Init == x = 0
Next == UNCHANGED x
NaturalCardinality(unused) == Cardinality(Nat)
IntegerCardinality(unused) == Cardinality(Int)
StringCardinality(unused) == Cardinality(STRING)
PowerValue(unused) == SUBSET Nat
PowerView == <<x, SUBSET Nat>>
PrintPower == PrintT(SUBSET Nat)
NaturalInvariant == Cardinality(Nat) = 0
PrintSecondError(unused) == Print(SUBSET Nat, 1 \div 0)
Checks(unused) == <<PrintT(Nat), PrintT(Seq(Nat)), PrintT({2, 1, 2}),
                   2 \in Print("test", Nat), Print("test", 7), ToString({2, 1, 2})>>
MixedSet(unused) == PrintT({1, TRUE})
UnionFunction(unused) == PrintT([n \in {1, 2} \cup {2, 3} |-> n])
UnionFunctionText(unused) == ToString([n \in UNION {{1, 2}, {2, 3}} |-> n])
TupleCardinality(unused) == Cardinality(<<1, 2>>)
EmptyTupleCardinality(unused) == Cardinality(<<>>)
RecordCardinality(unused) == Cardinality([a |-> 1])
FunctionCardinality(unused) == Cardinality([n \in 1..3 |-> n])
LambdaCardinality(unused) == Cardinality([n \in Nat |-> n])
EmptySequenceCardinality(unused) == Cardinality(Seq({}))
SequenceCardinality(unused) == Cardinality(Seq({1}))
ScalarCardinality(unused) == Cardinality(42)
SetCardinalities(unused) == <<Cardinality({}), Cardinality({1, 2, 1}),
    Cardinality(1..3), Cardinality(SUBSET {1, 2}), Cardinality([{1, 2} -> {3, 4}]),
    Cardinality({1, 2} \X {3}), Cardinality([a : {1, 2}]),
    Cardinality({n \in 1..3 : n > 1}), Cardinality(UNION {{1, 2}, {2, 3}})>>
====
