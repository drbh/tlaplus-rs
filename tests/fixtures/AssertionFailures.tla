---- MODULE AssertionFailures ----
EXTENDS Naturals, TLC
VARIABLE x
Init == x = 0
Next == x' = 1 - x
BadInit == Assert(FALSE, "init") /\ x = 0
BadNext == x' = 1 /\ Assert(FALSE, "next")
BadInvariant == Assert(x = 0, "invariant")
BadConstraint == Assert(x = 0, "constraint")
BadAction == Assert(x' = x, "action")
UndefinedNext == x' = 1 + <<1>>
Spec == Init /\ [][Next]_x
Property == [][BadAction]_x
====
