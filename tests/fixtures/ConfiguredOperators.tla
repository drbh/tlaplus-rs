---- MODULE ConfiguredOperators ----
EXTENDS Naturals, TLC
CONSTANTS F(_), G(_, _), Evaluate(_)
VARIABLE x
Apply(O(_), value) == O(value)
Init == x = 0
Next == /\ x' = F(x)
        /\ x' = Apply(F, x)
        /\ G(x, x) = x
Inv == x \in 0..2
Check == <<F(0), Apply(F, 1), G(1, 2), Evaluate({1, 2}), Apply(Evaluate, <<1, 2>>)>>
Undefined == F(99)
====
