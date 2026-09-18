---- MODULE IncompleteStates ----
EXTENDS Integers
VARIABLES z, x, y
Init == /\ x = 0 /\ y = 0 /\ z = 0
Next == UNCHANGED <<x, y, z>>
PartialInit == z = 0
PartialNext == z' = z + 1
====
