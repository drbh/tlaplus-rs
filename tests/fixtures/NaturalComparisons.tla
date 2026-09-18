---- MODULE NaturalComparisons ----
EXTENDS Naturals
VARIABLE x
Init == x = 0
Next == UNCHANGED x
Cast(unused) == TRUE + 1
HigherCast(Op(_, _), unused) == Op("a", 1)
CastCheck == Cast(x) = 0
HigherCastCheck == HigherCast(+, x) = 0
GeLeft == "a" >= "b"
GeRight == 1 >= FALSE
Alias(a, b) == a >= b
GeAlias == Alias("a", "b")
Higher(Cmp(_, _)) == Cmp("a", "b")
GeHigher == Higher(>=)
Valid == /\ 0 >= 0
         /\ 2 >= 1
         /\ ~(1 >= 2)
====
