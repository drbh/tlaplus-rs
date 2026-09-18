---- MODULE GuardedAction ----
EXTENDS Sequences
VARIABLE queue
Init == queue = <<>>
Remove == /\ queue # <<>>
          /\ queue' = Tail(queue)
Nested == /\ Remove
          /\ Head(queue) = 1
UnchangedGuard == /\ UNCHANGED queue
                  /\ FALSE
                  /\ queue' = Tail(queue)
Next == Nested \/ UnchangedGuard
====
