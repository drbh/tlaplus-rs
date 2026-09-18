---- MODULE AssumedRegisters ----
EXTENDS Integers, TLC
VARIABLE x
ASSUME TLCSet(0, 7)
ASSUME TLCSet("s:seed", {7})
ASSUME TLCGet(0) = 7
Init == x = TLCGet(0)
Next == /\ x < 9
        /\ TLCSet(0, x + 1)
        /\ x' = TLCGet(0)
Stable == x \in 7..9 /\ TLCGet("s:seed") = {7}
====
