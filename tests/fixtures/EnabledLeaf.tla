---- MODULE EnabledLeaf ----
EXTENDS Naturals
VARIABLES a, b
Action == a' = a /\ b' = b + 1
====
