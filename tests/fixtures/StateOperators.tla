---- MODULE StateOperators ----
EXTENDS Naturals
VARIABLES x, y
Possible(F(_)) == F(x' = x)
Impossible(F(_)) == F(x' = 0 /\ x' = 1)
Stable(F(_)) == F(x)
Primed(F(_)) == F(x) = x
Init == x = 0 /\ y = 0
Inv == /\ Possible(ENABLED)
       /\ ~Impossible(ENABLED)
Next == /\ x' = x /\ y' = 1 - y /\ Inv
Action == /\ Stable(UNCHANGED)
          /\ Primed(')
Spec == Init /\ [][Next]_<<x, y>>
Property == [][Action]_<<x, y>>
====
