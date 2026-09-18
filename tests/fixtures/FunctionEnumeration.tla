---- MODULE FunctionEnumeration ----
EXTENDS Integers, Sequences
VARIABLE x
Init == x = 0
Next == UNCHANGED x
Domain == {f \in [STRING -> {0}]: TRUE}
Range == {f \in [{0} -> Int]: TRUE}
Both == {f \in [STRING -> Int]: TRUE}
SequenceDomain == {f \in [Seq({}) -> {0}]: TRUE}
SequenceRange == {f \in [{0} -> Seq({})]: TRUE}
NestedDomain == {f \in [{[STRING -> {0}]} -> {0}]: TRUE}
EmptyDomain == {f \in [{} -> Int]: TRUE}
EmptyRange == {f \in [STRING -> {}]: TRUE}
IntervalEmpty == {f \in [1..0 -> Int]: TRUE}
IntervalRange == {f \in [0..1 -> Int]: TRUE}
Finite == {f \in [{0, 1} -> {2, 3}]: TRUE}
RecordField == {r \in [a: {0}, b: Int]: TRUE}
RecordOrder == {r \in [z: Int, a: STRING]: TRUE}
RecordSequence == {r \in [a: Seq({})]: TRUE}
RecordEmpty == {r \in [a: Int, b: {}]: TRUE}
RecordFinite == {r \in [a: {0}, b: {1, 2}]: TRUE}
ProductFirst == {p \in (Int \X {0}): TRUE}
ProductSecond == {p \in ({0} \X STRING): TRUE}
ProductBoth == {p \in (Int \X STRING): TRUE}
ProductSequence == {p \in ({0} \X Seq({})): TRUE}
ProductEmpty == {p \in (Int \X {}): TRUE}
ProductFinite == {p \in ({0} \X {1, 2}): TRUE}
====
