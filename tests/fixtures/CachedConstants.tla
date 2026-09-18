---- MODULE CachedConstants ----
EXTENDS Integers, TLC
CONSTANT C
VARIABLE x
Pick == RandomElement(1..100)
Before == RandomElement(1..100)
Later == RandomElement(1..100)
Copy == Later
After == RandomElement(1..100)
UnusedBad == RandomElement(1..100) \div 0
ASSUME RandomElement({TRUE})
Init == x = RandomElement(1..100)
Next == x' = x /\ FALSE
Inv == /\ Pick = 74
       /\ Before = 42
       /\ Copy = 21
       /\ Later = 21
       /\ After = 34
       /\ C = 94
       /\ C = 95
       /\ x = 97
====
