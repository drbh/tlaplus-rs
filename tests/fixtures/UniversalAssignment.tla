---- MODULE UniversalAssignment ----
EXTENDS Naturals
VARIABLE x
Init == \/ (\A n \in 1..1000 : x = 1)
        \/ (\A n \in {0, 1} : x = n)
        \/ ((\A n \in {} : x = n) /\ x = 2)
Next == UNCHANGED x
Inv == x \in {1, 2}
====
