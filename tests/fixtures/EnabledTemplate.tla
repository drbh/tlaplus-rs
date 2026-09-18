---- MODULE EnabledTemplate ----
EXTENDS Naturals
VARIABLES u, v
Action == u' = u /\ v' = v + 1
Direct == ENABLED Action
Check(a) == ENABLED a
Captured == Check(Action)
LetCaptured == LET A == Action IN ENABLED A
CapturedLater == ENABLED (LET A == Action IN A)
Nested == ENABLED (Action /\ ENABLED (u' = 17 /\ v' = 18))
Impossible == ENABLED (u' = 0 /\ u' = 1)
Distinct == ENABLED (u' = 0 /\ v' = 1)
CurrentDistinct == ENABLED (u # v /\ Action)
Stutter == ENABLED (UNCHANGED <<u>> /\ v' = v + 1)
RECURSIVE Step(_)
Step(n) == IF n = 0 THEN Action ELSE Step(n - 1)
Recursive == ENABLED Step(2)
RECURSIVE Nest(_)
Nest(n) == IF n = 0 THEN TRUE ELSE ENABLED (Action /\ Nest(n - 1))
RecursiveNested == Nest(2)
====
