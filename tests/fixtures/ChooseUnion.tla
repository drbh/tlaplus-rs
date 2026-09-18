---- MODULE ChooseUnion ----
EXTENDS Naturals
VARIABLE x
Init == x = CHOOSE i \in {2} \cup {0}: TRUE
Next == UNCHANGED x
Inv == x = 0
====
