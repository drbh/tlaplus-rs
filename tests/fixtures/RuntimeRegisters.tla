---- MODULE RuntimeRegisters ----
EXTENDS Integers, TLC, TLCExt
VARIABLE x
Init == /\ x = 0
        /\ TLCSet(0, TLCGetOrDefault(0, x))
        /\ TLCSet("s:seen", IF x = 0 THEN {} ELSE {x})
Next == /\ x < 3
        /\ TLCSet(0, TLCGetOrDefault(0, 1000) + 1)
        /\ TLCSet("s:seen", TLCGet("s:seen") \cup {x})
        /\ x' = x + 1
Consistent == TLCGet(0) = x /\ TLCGet("s:seen") = 0..(x - 1)
Bound == TLCGet(0) < 2
====
