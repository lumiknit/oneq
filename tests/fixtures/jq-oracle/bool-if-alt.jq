# Comparisons, boolean and/or/not, if-then-elif-else, alternative operator (//)
{
  eq_check: (.n == 5),
  cmp_check: (.n > 0),
  and_or: [(.a and .b), (.a or .b), ((.a and .b) | not)],
  if_chain: (if .n > 10 then "big" elif .n > 0 then "small-pos" elif .n == 0 then "zero" else "neg-or-null" end),
  alt_default: (.x // "default"),
  alt_chain: (.a // .b // "fallback"),
  not_x: (.x | not)
}
