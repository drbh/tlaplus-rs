---- MODULE AnyMembership ----
EXTENDS Naturals, TLC, FiniteSets
VARIABLE x
Init == x = 0
Next == x' = 1 - x
Inv == /\ {x, x + 1} \subseteq Any
       /\ ~IsFiniteSet(Any)
       /\ ToString(Any) = "ANY"
====
