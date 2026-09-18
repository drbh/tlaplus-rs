---- MODULE Counter ----
EXTENDS Naturals
VARIABLE x
Init == x = 0
Next == x' = IF x < 3 THEN x + 1 ELSE x
Stop == x < 3 /\ x' = x + 1
Safe == x \in 0..3
Broken == x < 2
InitiallyBroken == x > 0
NoChange == [][x' = x]_x
EventuallyAlwaysZero == <>[](x = 0)
Spec == Init /\ [][Next]_x
====
