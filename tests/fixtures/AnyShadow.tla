---- MODULE AnyShadow ----
EXTENDS Naturals
VARIABLE x
Any == {x}
JavaTime == x
TLCModelValue(v) == v + x
Init == x = 0
Next == x' = 1 - x
Inv == x \in Any /\ (x + 1) \notin Any /\ JavaTime = x /\ TLCModelValue(1) = x + 1
====
