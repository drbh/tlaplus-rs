---- MODULE Bank ----
(* lesson 6: functions and records.

   a function maps a set to values: [a \in Accounts |-> 10] gives every
   account 10. balance[a] reads one entry. EXCEPT builds a copy with some
   entries changed, and @ is the old value at that entry. a function with
   only balance[from]' = ... would leave the other entries unconstrained, so
   you must always update the whole function.

   a record [from |-> "alice", amt |-> 2] is a function whose domain is a set
   of strings, read with last.amt or last["amt"]. a sequence <<1, 2>> is a
   function on 1..2. [Accounts -> 0..Total] is the set of all functions from
   Accounts to 0..Total, and [from: Accounts, amt: 1..Limit] the set of all
   such records, both handy in type invariants.

   run:    tla examples/06-functions/Bank.tla
   expect: Safety checks passed                                              *)
EXTENDS Integers

Accounts == {"alice", "bob", "carol"}
Limit == 3
Total == 3 * Limit

VARIABLES balance, last

Init ==
  /\ balance = [a \in Accounts |-> Limit]
  /\ last = [from |-> "alice", to |-> "alice", amt |-> 0]

Transfer(from, to, amt) ==
  /\ from # to
  /\ balance[from] >= amt
  /\ balance' = [balance EXCEPT ![from] = @ - amt, ![to] = @ + amt]
  /\ last' = [from |-> from, to |-> to, amt |-> amt]

Next == \E from, to \in Accounts, amt \in 1..Limit : Transfer(from, to, amt)

TypeOK ==
  /\ balance \in [Accounts -> 0..Total]
  /\ last \in [from: Accounts, to: Accounts, amt: 0..Limit]
  /\ DOMAIN balance = Accounts

NoOverdraft == \A a \in Accounts : balance[a] >= 0

Conserved == balance["alice"] + balance["bob"] + balance["carol"] = Total
====
---- CONFIG Bank ----
INIT Init
NEXT Next
INVARIANT TypeOK
INVARIANT NoOverdraft
INVARIANT Conserved
====
