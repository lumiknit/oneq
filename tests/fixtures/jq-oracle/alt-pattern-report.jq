#! -n
# Retry through filter arguments, closures, reduce/foreach, and object fields.
def unpack: . as {payload: $p} ?// $p | $p;
def with_record(f): unpack as $record | $record | f;
def total: reduce .items[] as $x (0; . + $x);
[
  {items: [2, 3, 5]},
  {payload: {items: [7, 11]}},
  {items: []}
]
| map(with_record(
    total as $total
    | {
        total: $total,
        running: [foreach .items[] as $x (0; . + $x; .)],
        checks: [.items[] | try (if . < 4 then error("small") else . end) catch .],
        scaled: [.items[] | . * $total]
      }
  ))
| {records: ., grand_total: (map(.total) | add)}
