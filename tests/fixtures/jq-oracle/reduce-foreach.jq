# reduce and foreach, in simple and more elaborate (extract) forms.
{
  sum_reduce: (reduce .nums[] as $n (0; . + $n)),
  max_reduce: (reduce .nums[] as $n (null; if . == null or $n > . then $n else . end)),
  running_totals: [foreach .nums[] as $n (0; . + $n)],
  foreach_extract: [foreach .nums[] as $n (0; . + $n; {value: $n, running: .})],
  count_positive: (reduce .nums[] as $n (0; if $n > 0 then . + 1 else . end))
}
