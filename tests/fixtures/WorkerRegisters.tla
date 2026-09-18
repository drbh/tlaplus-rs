---- MODULE WorkerRegisters ----
EXTENDS Integers, TLC, TLCExt
VARIABLES x, y
Init == /\ x \in 1..4
        /\ y = 0
        /\ TLCSet(0, y)
        /\ TLCSet("s:step", y)
Next == /\ y = 0
        /\ TLCSet("s:step", y)
        /\ \E k \in 1..2 : /\ TLCGet("s:step") = k - 1
                            /\ TLCSet(0, k)
                            /\ x' = x
                            /\ y' = k
Consistent == y = 0 \/ (TLCGet(0) = y /\ TLCSet("s:step", y))
====
