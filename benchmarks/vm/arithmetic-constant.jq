# Same steady-state workload as c.sh; exercises literal infix operands.
reduce range(1; 8000001) as $i
  (0;
    ($i * 3 + 7) as $x
    | if $x % 5 == 0 then
        . + ($x / 5 | floor)
      elif $x % 3 == 0 then
        . - $x
      else
        . + ($x % 97)
      end
  )
