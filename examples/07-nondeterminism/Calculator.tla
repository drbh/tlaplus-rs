---- MODULE Calculator ----
(* lesson 7: nondeterminism.

   a user types NumInputs digits into a calculator and picks + - or * each
   time. \E x \in Digits says "some digit", \/ says "one of these branches",
   and the checker explores every combination. nondeterminism is how a spec
   abstracts over users, schedulers, random values and failures.

   the "invariant" NotTarget is really a question: can the display reach
   Target? the checker answers by violating it and printing how.

   run:    tla examples/07-nondeterminism/Calculator.tla
   expect: Invariant NotTarget is violated.
           a trace of 5 inputs ending at sum = 417, like
           0 -> 1 -> 10 -> 60 -> 420 -> 417, (((0+1)+9)*6)*7-3

   try Target = 851, the smallest number five inputs cannot reach          *)
EXTENDS Integers

CONSTANTS NumInputs, Target
ASSUME NumInputs \in Nat /\ Target \in Int

Digits == 0..9

VARIABLES sum, inputs

Init == sum = 0 /\ inputs = 0

Input ==
  /\ inputs < NumInputs
  /\ \E x \in Digits :
       \/ sum' = sum + x
       \/ sum' = sum - x
       \/ sum' = sum * x
  /\ inputs' = inputs + 1

Done == inputs = NumInputs /\ UNCHANGED <<sum, inputs>>

Next == Input \/ Done

NotTarget == sum # Target
====
---- CONFIG Calculator ----
CONSTANTS
  NumInputs = 5
  Target = 417
INIT Init
NEXT Next
INVARIANT NotTarget
====
