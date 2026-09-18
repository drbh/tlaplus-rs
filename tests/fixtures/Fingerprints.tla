---- MODULE Fingerprints ----
EXTENDS Integers, TLC, TLCExt
CONSTANT M
VARIABLE x
Init == x = [k \in {<<1>>, <<2>>} |-> 0]
Next == UNCHANGED x
Inv == TLCFP([x EXCEPT ![<<1>>] = 10]) = TLCFP((<<1>> :> 10) @@ (<<2>> :> 0))
Check == <<TLCFP(TRUE), TLCFP(FALSE), TLCFP(-123), TLCFP("abc"), TLCFP("λ🚀"),
           TLCFP(M), TLCFP({M, 1, -1}), TLCFP(<<1, 2>>), TLCFP([a |-> 1, b |-> 2]),
           TLCFP([k \in 1..3 |-> k * k]), TLCFP(SUBSET (1..3))>>
====
