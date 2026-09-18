---- MODULE BooleanContexts ----
EXTENDS Naturals, TLC
CONSTANT M
VARIABLE x
Init == x = 0
Next == x' = x + 1
StringIf(unused) == IF "bad" THEN TRUE ELSE FALSE
StringNot(unused) == ~"bad"
ModelIf(unused) == IF M THEN TRUE ELSE FALSE
ModelNot(unused) == ~M
MemberIf(unused) == 1 \in (IF 1 THEN {1} ELSE {})
RenderIf(unused) == ToString(IF 1 THEN {1} ELSE {}) = "{}"
StringIfCheck == StringIf(x)
StringNotCheck == StringNot(x)
ModelIfCheck == ModelIf(x)
ModelNotCheck == ModelNot(x)
MemberIfCheck == MemberIf(x)
RenderIfCheck == RenderIf(x)
InvalidInit == IF 1 THEN x = 0 ELSE x = 1
InvalidNext == IF "bad" THEN x' = 1 ELSE x' = 2
EnabledIfCheck == ENABLED (IF M THEN x' = 1 ELSE x' = 2)
Change[n \in {0, 1}] == x' = n
Selected == IF M THEN Change ELSE Change
InvalidFunctionNext == Selected[1]
====
