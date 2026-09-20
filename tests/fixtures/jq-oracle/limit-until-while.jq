# limit, first/last/nth, until, while, repeat.
(if .n < 0 then 0 else .n end) as $n
| {
  limited: [limit(3; range(0; 1000))],
  first_val: [range($n)] | first,
  last_val: [range($n)] | last,
  nth_val: ([range($n)] | (if length > 2 then nth(2; .[]) else null end)),
  while_vals: [1 | while(. < $n; . * 2)],
  until_val: (1 | until(. >= $n; . + 1)),
  repeat_capped: [limit(4; 1 | repeat(. * 2))]
}
