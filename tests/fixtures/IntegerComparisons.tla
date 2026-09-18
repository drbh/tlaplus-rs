---- MODULE IntegerComparisons ----
EXTENDS Integers
VARIABLE x
Init == x = 0
Next == UNCHANGED x
Cast(unused) == TRUE + 1
HigherCast(Op(_, _), unused) == Op("a", 1)
CastCheck == Cast(x) = 0
HigherCastCheck == HigherCast(+, x) = 0
LtLeft == <<1>> < <<2>>
LtRight == 1 < "a"
LeLeft == FALSE <= TRUE
LeRight == 1 <= {2}
GtLeft == <<1, 1>> > 0
GtRight == 1 > "a"
GeLeft == "a" >= "b"
GeRight == 1 >= FALSE
Valid == /\ -1 < 0
         /\ 0 <= 0
         /\ 1 > -1
         /\ 1 >= 1
====
