# Whitespace-sensitive edge cases for the min/oneline formatter:
# unary minus butting up against another minus or a negative
# literal, plus "and"/"or" keyword boundaries right after operands and
# a multi-branch conditional (`elif`).

((-1) - -1) as $a |
(1 - -$a) as $b |
if $a > 0 and $b >= 0 then "positive"
elif $a == 0 then "zero"
else "negative"
end
