---- MODULE RecursiveOperators ----
EXTENDS Naturals, Sequences
VARIABLE x
RECURSIVE Fold(_, _, _)
Fold(F(_, _), acc, q) == IF q = <<>> THEN acc
                        ELSE Fold(F, F(acc, Head(q)), Tail(q))
Twice(F(_, _), q) == Fold(LAMBDA a, b : F(F(a, b), b), 0, q)
RECURSIVE Sum(_), ShiftSum(_)
Sum(q) == IF q = <<>> THEN 0 ELSE Head(q) + Sum(Tail(q))
ShiftSum(q) == IF q = <<>> THEN 0 ELSE Head(q) + x + ShiftSum(Tail(q))
Init == x = 0
Next == /\ x' = 1 - x
        /\ Sum(<<x, 1, 2>>)' = 4 - x
        /\ Fold(+, 0, <<x, 1, 2>>)' = 4 - x
        /\ ShiftSum(<<1, 2>>)' = 5 - 2 * x
Inv == /\ Fold(+, 0, <<x, 1, 2>>) = x + 3
       /\ Fold( *, 1, <<x, 2, 3>>) = 6 * x
       /\ Fold(LAMBDA a, b : a + b + x, 0, <<1, 2>>) = 3 + 2 * x
       /\ \A k \in {1, 2} : Fold(LAMBDA a, b : a + b + k, 0, <<1, 2>>) = 3 + 2 * k
       /\ Twice(+, <<1, 2>>) = 6
====
