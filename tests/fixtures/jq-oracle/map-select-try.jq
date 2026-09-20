# map, map_values, select, empty, error/try-catch, and the ?// alternative
# destructuring-pattern operator.
{
  doubled: (.items | map(if type == "number" then . * 2 else . end)),
  evens_only: (.items | map(select(type == "number" and . % 2 == 0))),
  drop_negatives: [.items[] | select(type == "number" and . >= 0)],
  skip_via_empty: [.items[] | if type == "number" and . < 0 then empty else . end],
  v_or_zero: [.items[] | (.v? // 0)],
  caught_errors: [.items[] | try (if type == "string" then error("bad-item: \(.)") else . end) catch .],
  alt_pattern: [.items[] as [$a] ?// $a | $a]
}
