# Summarize a list of {name, score} records: total, count, pass
# count (>= 60) and average. Exercises reduce + object-pattern
# destructuring + object construction + if/then/else.

reduce .[] as {name: $n, score: $s} (
  {total: 0, count: 0, passed: 0};
  {
    total: (.total + $s),
    count: (.count + 1),
    passed: (.passed + (if $s >= 60 then 1 else 0 end))
  }
) | . + {average: (if .count > 0 then .total / .count else 0 end)}
