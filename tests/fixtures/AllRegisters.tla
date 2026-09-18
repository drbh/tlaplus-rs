---- MODULE AllRegisters ----
EXTENDS Integers, Sequences, TLC
VARIABLE x
Init == /\ x = 0
        /\ TLCSet(7, x + 10)
        /\ TLCSet("s:fixed", x + 20)
        /\ TLCSet(0, x)
Next == /\ x < 3
        /\ TLCSet(0, x + 1)
        /\ x' = TLCGet(0)
Consistent == LET indexed == TLCGet("all")
                  named == TLCGet("all:named")
              IN /\ DOMAIN indexed = {0, 7}
                 /\ DOMAIN named = {"s:fixed"}
                 /\ Len(indexed[0]) \in {1, 4}
                 /\ DOMAIN indexed[0] = DOMAIN indexed[7]
                 /\ DOMAIN indexed[0] = DOMAIN named["s:fixed"]
                 /\ \A w \in DOMAIN indexed[0] :
                      /\ indexed[0][w] \in 0..3
                      /\ indexed[7][w] = 10
                      /\ named["s:fixed"][w] = 20
====
