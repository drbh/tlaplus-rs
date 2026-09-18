---- MODULE AssertErrors ----
EXTENDS Naturals, Sequences, TLC, TLCExt
CONSTANTS Message, Catch(_, _)
VARIABLE x
Init == x = 0
Next == x' = 1 - x
Inv == ~AssertError("no error", x) /\ (x = 1 \/ AssertError("The second argument of \\div is 0.", 1 \div x))
Checks == /\ AssertError("Attempted to apply Head to the empty sequence.", Head(<<>>))
          /\ AssertError("Attempted to apply Tail to the empty sequence.", Tail(<<>>))
          /\ ~AssertError("wrong message", 1 \div 0)
          /\ AssertError("The second argument of % should be a positive number, but instead it is:\n0", 1 % 0)
          /\ AssertError("The first argument of Assert evaluated to FALSE; the second argument was:\n\"payload\"", Assert(FALSE, "payload"))
AliasNoError == Catch("no error", TRUE)
NonLiteral == AssertError(Message, Assert(FALSE, "must not evaluate"))
ComputedMessage == AssertError("a" \o "b", TRUE)
Wrapped(s) == AssertError(s, TRUE)
WrappedCheck == Wrapped("literal at call site")
UnknownText == AssertError("unknown", 1 + "a")
Take(v) == AssertError("The second argument of \\div is 0.", 1 \div v)
Bindings == \A i \in 0..1: Take(i) = (i = 0)
PrimeWorks == Take(x)' = (x' = 0)
Use(F(_, _)) == F("no error", TRUE)
HigherCheck == Use(AssertError)
Forward(F(_, _), msg) == F(msg, TRUE)
ForwardCheck == Forward(AssertError, "literal at call site")
====
