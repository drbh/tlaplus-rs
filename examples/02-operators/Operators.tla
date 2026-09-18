---- MODULE Operators ----
(* lesson 2: operators and values.

   operators are defined with == and are the building blocks of every spec.
   nothing here changes over time, so each fact is an ASSUME: a constant
   expression the checker evaluates once before exploring any states. change
   one to see it fail.

   run:    tla examples/02-operators/Operators.tla
   expect: Safety checks passed                                              *)
EXTENDS Integers, Sequences, FiniteSets

\* operators take a fixed number of arguments. IF always needs an ELSE
MinutesToSeconds(m) == m * 60
Abs(n) == IF n < 0 THEN -n ELSE n
ASSUME MinutesToSeconds(2) = 120
ASSUME Abs(-3) = 3

\* booleans: /\ and, \/ or, ~ not, => implies, <=> iff
Xor(A, B) == A = ~B
ASSUME Xor(TRUE, FALSE) /\ ~Xor(TRUE, TRUE)
ASSUME (FALSE => TRUE) /\ ((TRUE => FALSE) = FALSE)

\* sequences are 1-indexed, written <<a, b, c>>
s == <<"a">>
ASSUME Append(s, "b") = <<"a", "b">>
ASSUME s \o <<"b", "c">> = <<"a", "b", "c">>
ASSUME Head(<<1, 2, 3>>) = 1 /\ Tail(<<1, 2, 3>>) = <<2, 3>>
ASSUME Len(s) = 1 /\ SubSeq(<<1, 3, 5>>, 1, 2) = <<1, 3>>

\* sets are unordered and unique
ASSUME 2 \in {1, 2, 3} /\ 4 \notin {1, 2, 3}
ASSUME {1, 2} \union {2, 3} = {1, 2, 3}
ASSUME {1, 2} \intersect {2, 3} = {2}
ASSUME {1, 2, 3} \ {2} = {1, 3}
ASSUME {1, 2} \subseteq {1, 2, 3} /\ Cardinality({1, 2, 3}) = 3

\* sets of values: ranges, map, filter, products, powersets
ASSUME 1..3 = {1, 2, 3} /\ 3..1 = {}
ASSUME {n * n : n \in 1..4} = {1, 4, 9, 16}
ASSUME {n \in 1..4 : n % 2 = 0} = {2, 4}
ASSUME <<1, TRUE>> \in (1..2) \X BOOLEAN
ASSUME SUBSET {1, 2} = {{}, {1}, {2}, {1, 2}}

Range(seq) == {seq[i] : i \in 1..Len(seq)}
ASSUME Range(<<3, 1, 3>>) = {1, 3}

\* quantifiers: \A for all, \E there exists. \A over {} is vacuously true
ASSUME \A n \in 1..5 : n > 0
ASSUME \E n \in 1..5 : n = 3
ASSUME \A n \in {} : FALSE

\* CHOOSE picks the one element satisfying a predicate, LET names local
\* definitions. with no matching element CHOOSE is an error
ClockType == (0..23) \X (0..59) \X (0..59)
ToSeconds(t) == t[1] * 3600 + t[2] * 60 + t[3]
ToClock(seconds) ==
  LET perDay == 86400
  IN CHOOSE t \in ClockType : ToSeconds(t) = seconds % perDay
ASSUME ToClock(3661) = <<1, 1, 1>>

\* a spec still needs a state, so here is the smallest one
VARIABLE unused
Init == unused = 0
Next == UNCHANGED unused
====
---- CONFIG Operators ----
INIT Init
NEXT Next
====
