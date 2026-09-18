---- MODULE ConstantDependencies ----
EXTENDS Integers, TLC
CONSTANTS A, B
VARIABLE x
Pick == RandomElement(1..100)
Derived == A + RandomElement(1..100)
After == RandomElement(1..100)
ASSUME RandomElement({TRUE})
Init == x = RandomElement(1..100)
Next == x' = x /\ FALSE
Inv == /\ Pick = 74
       /\ Derived = 63
       /\ After = 34
       /\ A = 97
       /\ B = 189
       /\ x = 1
====
