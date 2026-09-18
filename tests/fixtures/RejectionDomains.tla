---- MODULE RejectionDomains ----
EXTENDS Integers, Sequences
VARIABLE x
Init == x = 0
Next == x' = 1 - x
BadHead == x = Head(<<>>)
BadTail == x' = Tail(<<>>)
BadChoose == (CHOOSE i \in {0, 1}: i > 1) = 0
BadCase == CASE FALSE -> TRUE
BadQuantifier == \A i \in Nat : i >= 0
BadSequence == \E q \in {s \in Seq(0..1): Len(s) <= 2}: x' = Len(q)
BadInitial == \E i \in Nat: x = i
BadDivision == x' = 1 \div 0
BadModulusZero == 1 % 0 = 0
BadModulusNegative == 1 % (-2) = 0
====
