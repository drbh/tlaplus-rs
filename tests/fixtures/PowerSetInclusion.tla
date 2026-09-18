---- MODULE PowerSetInclusion ----
EXTENDS Integers
VARIABLE b
Init == b = TRUE
Next == b' = /\ (SUBSET (1..23) \subseteq SUBSET (1..42))
             /\ ~(SUBSET (1..42) \subseteq SUBSET (1..23))
             /\ ~(SUBSET (1..42) \subseteq SUBSET (2..42))
             /\ (SUBSET (2..42) \subseteq SUBSET (1..42))
             /\ (SUBSET {} \subseteq SUBSET {})
             /\ (SUBSET {} \subseteq SUBSET {1})
             /\ ~(SUBSET {1} \subseteq SUBSET {})
             /\ (SUBSET {1} \subseteq SUBSET Int)
             /\ (SUBSET {1} \subseteq SUBSET Nat)
Inv == b = TRUE
====
