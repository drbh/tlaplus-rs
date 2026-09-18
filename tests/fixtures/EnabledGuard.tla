---- MODULE EnabledGuard ----
EXTENDS Naturals, TLC
VARIABLE x
Init == x = 0
Next == \/ /\ ~ENABLED (x' = x)
           /\ Assert(FALSE, "disabled action")
           /\ UNCHANGED x
        \/ x' = 1 - x
Inv == /\ x \in {0, 1}
       /\ ~ENABLED <<x' = x>>_x
       /\ ENABLED <<x' = 1 - x>>_x
====
