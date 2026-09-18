---- MODULE UnexpandedSnapshot ----
EXTENDS TLC
VARIABLE x
Init == x \in {0, 1}
Next == Assert(FALSE, "must not replay an unexpanded state") /\ x' = x
Inv == x = 0
====
