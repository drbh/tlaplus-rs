---- MODULE RandomConstants ----
EXTENDS Integers, TLC
CONSTANT C
VARIABLE x
Pick == RandomElement(1..100)
ASSUME RandomElement({TRUE})
Init == x = RandomElement(1..100)
Next == x' = x /\ FALSE
Inv == C = 34 /\ C = 97 /\ x = 21
====
