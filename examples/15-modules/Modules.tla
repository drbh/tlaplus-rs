---- MODULE Modules ----
(* lesson 15: modules.

   EXTENDS pulls every definition of a module into this one. INSTANCE also
   brings a module in but behind a name: Seq!Append avoids clashes and reads
   as "Append from Sequences".

   a module with CONSTANTS is given values at INSTANCE time with WITH. Point
   (in Point.tla next to this file) has X and Y, and each instance below
   fixes both. a module can be instantiated as many times as you like.

   note: TLC also allows partially parameterized instances such as
   XAxis(X) == INSTANCE Point WITH Y <- 0, used as XAxis(2)!Repr. tla does
   not support those yet

   run:    tla examples/15-modules/Modules.tla
   expect: Safety checks passed                                              *)
EXTENDS Integers

Seq == INSTANCE Sequences
ASSUME Seq!Append(<<1>>, 2) = <<1, 2>>

Origin == INSTANCE Point WITH X <- 0, Y <- 0
Unit == INSTANCE Point WITH X <- 1, Y <- 1

ASSUME Origin!Repr = <<0, 0>>
ASSUME Origin!Add(1, 2) = <<1, 2>>
ASSUME Unit!Add(1, 2) = <<2, 3>>

\* walk from the origin along the diagonal, one Unit step at a time
VARIABLE point

Init == point = Origin!Repr

Step == point[1] < 3 /\ point' = Unit!Add(point[1], point[2])

Stay == point[1] = 3 /\ UNCHANGED point

Next == Step \/ Stay

OnDiagonal == point[1] = point[2] /\ point[1] \in 0..3
====
---- CONFIG Modules ----
INIT Init
NEXT Next
INVARIANT OnDiagonal
====
