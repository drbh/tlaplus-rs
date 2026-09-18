---- MODULE OperatorShadow ----
VARIABLE x
a + b == a
a \o b == b
Len(a) == a
Init == x = 0
Next == x' = IF x = 0 THEN 1 ELSE 0
Inv == /\ (x + 3) = x
       /\ (x \o 3) = 3
       /\ Len(x) = x
====
