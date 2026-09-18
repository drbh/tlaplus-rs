---- MODULE RandomModel ----
EXTENDS Integers, TLC
CONSTANTS A, B
VARIABLES x, n, tag
ASSUME RandomElement({TRUE})
Init == /\ x = RandomElement(1..100)
        /\ n = 0
        /\ tag \in {A, B}
Next == /\ n' = n + 1
        /\ tag' = IF tag = A THEN B ELSE A
        /\ (x' = RandomElement(1..100) \/ x' = RandomElement(1..100))
Inv == n >= 0 /\ RandomElement(1..10) > 0
FailingInv == n = 0 \/ RandomElement(1..10) > 1
ActionFilter == n' > n /\ RandomElement(1..10) > 0
StateFilter == n < 3
Projection == <<n, x, tag>>
Sym == Permutations({A, B})
====
