---- MODULE EnabledInstances ----
EXTENDS Naturals
VARIABLE x
I == INSTANCE EnabledTemplate WITH u <- x, v <- x
J == INSTANCE EnabledTemplate WITH u <- x, v <- x + 1
Init == x = 0
Next == I!Step(0) \/ x' = 1 - x
Inv == /\ I!Direct
       /\ ~I!Captured
       /\ ~I!LetCaptured
       /\ I!Distinct
       /\ ~I!CurrentDistinct
       /\ I!Stutter
       /\ I!Recursive
       /\ ~ENABLED I!Action
       /\ I!CapturedLater
       /\ I!Nested
       /\ ~I!Impossible
       /\ I!RecursiveNested
       /\ J!Direct
       /\ J!CurrentDistinct
       /\ J!Stutter
       /\ ~J!LetCaptured
====
