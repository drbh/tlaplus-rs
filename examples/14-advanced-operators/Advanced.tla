---- MODULE Advanced ----
(* lesson 14: more operators.

   recursion, higher-order operators, LAMBDA, CASE and recursive functions.
   like lesson 2 everything is constant, so the facts are ASSUMEs.

   run:    tla examples/14-advanced-operators/Advanced.tla
   expect: Safety checks passed                                              *)
EXTENDS Integers, Sequences, FiniteSets

\* RECURSIVE declares an operator before its definition so it can call
\* itself. nothing checks that the recursion ends
RECURSIVE SumSeq(_)
SumSeq(s) == IF s = <<>> THEN 0 ELSE Head(s) + SumSeq(Tail(s))
ASSUME SumSeq(<<1, 2, 3>>) = 6

\* recursing over a set: CHOOSE picks an element, the rest is the set minus it
RECURSIVE SetSum(_)
SetSum(set) ==
  IF set = {} THEN 0
  ELSE LET x == CHOOSE x \in set : TRUE
       IN x + SetSum(set \ {x})
ASSUME SetSum({1, 2, 3, 4}) = 10

\* when order matters, choose deterministically: the smallest each time
RECURSIVE SetToSeq(_)
SetToSeq(set) ==
  IF set = {} THEN <<>>
  ELSE LET x == CHOOSE x \in set : \A y \in set : x <= y
       IN <<x>> \o SetToSeq(set \ {x})
ASSUME SetToSeq({3, 1, 2}) = <<1, 2, 3>>

\* operators can take operators. Op(_) is a one-argument operator parameter,
\* LAMBDA writes one inline. recursive and higher-order cannot be combined
SeqMap(Op(_), s) == [i \in DOMAIN s |-> Op(s[i])]
Double(n) == n * 2
ASSUME SeqMap(Double, <<1, 2>>) = <<2, 4>>
ASSUME SeqMap(LAMBDA n : n + 1, <<1, 2>>) = <<2, 3>>

\* a user defined infix operator
set ++ x == set \union {x}
ASSUME ({1} ++ 2) = {1, 2}

\* CASE: the first matching branch wins, OTHER is the fallback and leaving it
\* out is an error when nothing matches
Sign(n) ==
  CASE n > 0 -> 1
    [] n < 0 -> -1
    [] OTHER -> 0
ASSUME Sign(5) = 1 /\ Sign(-5) = -1 /\ Sign(0) = 0

\* a recursive function. f[x \in S] == e is a function, not an operator, so
\* it can refer to itself without RECURSIVE
Factorial[n \in 0..10] == IF n = 0 THEN 1 ELSE n * Factorial[n - 1]
ASSUME Factorial[5] = 120

\* :> and @@ from TLC build and merge functions one entry at a time
LOCAL INSTANCE TLC
ASSUME ((1 :> "a") @@ (2 :> "b")) = <<"a", "b">>

VARIABLE unused
Init == unused = 0
Next == UNCHANGED unused
====
---- CONFIG Advanced ----
INIT Init
NEXT Next
====
