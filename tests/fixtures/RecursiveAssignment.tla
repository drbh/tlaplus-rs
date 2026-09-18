---- MODULE RecursiveAssignment ----
EXTENDS Naturals
VARIABLE x
RECURSIVE Assign(_, _, _)
Assign(v, value, n) == IF n = 0 THEN v = value ELSE Assign(v, value, n - 1)
Init == Assign(x, 0, 3)
Next == Assign(x', 1 - x, 3)
Inv == x \in {0, 1}
====
