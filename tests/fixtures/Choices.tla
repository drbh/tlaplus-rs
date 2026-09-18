---- MODULE Choices ----
EXTENDS Naturals, TLC
CONSTANTS Limit, Labels
VARIABLES x, label
Bound == Limit
Ceiling == 2
Init == /\ x \in {0, 1}
        /\ label \in Labels
Advance == /\ x < Bound
           /\ x' = x + 1
           /\ UNCHANGED label
Reset == /\ x > 0
         /\ x' = 0
         /\ UNCHANGED label
Stay == UNCHANGED <<x, label>>
Next == Advance \/ Reset \/ Stay \/ Advance
TypeOK == x \in 0..Bound /\ label \in Labels
KeepState == x < 1
KeepStep == x' # 0
Sym == Permutations(Labels)
View == x
====
