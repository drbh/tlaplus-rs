---- MODULE InvalidSelection ----
EXTENDS Integers
VARIABLE x
Init == x = 0
Next == x' = x + 1
TupleIndex(unused) == <<1, 2>>[0]
RecordField(unused) == [a |-> 1].b
AppliedInterval(unused) == (1..3)[1]
FunctionDomain(unused) == [i \in 1..2 |-> i][3]
IgnoredUpdate(unused) == [[i \in 1..2 |-> IF i = 1 THEN 42 ELSE 1 \div 0]
                         EXCEPT ![3] = 0][1]
TupleCheck == TupleIndex(x) = 1
RecordCheck == RecordField(x) = 1
IntervalCheck == AppliedInterval(x) = 1
FunctionCheck == /\ IgnoredUpdate(x) = 42
                 /\ FunctionDomain(x) = 1
====
