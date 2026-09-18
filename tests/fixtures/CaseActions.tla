---- MODULE CaseActions ----
EXTENDS Naturals, TLC
VARIABLES x, y
Init == CASE TRUE -> x = 0 /\ y = 0 [] OTHER -> Assert(FALSE, "unselected branch")
Next == CASE x = 0 -> x' = 1 /\ y' = x'
          [] x = 1 -> x' = 2 /\ y' = x'
          [] OTHER -> x' = 0 /\ y' = x'
Inv == x = y /\ x \in 0..2
====
