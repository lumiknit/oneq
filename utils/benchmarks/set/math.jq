. as $n
| [
    range($n)
    | . as $i
    | [
        range(20)
        | . as $j
        | (($i + $j + 1) / 10) as $x
        | (
            (($x | sin) * ((-$x / 50) | exp))
            + (($x + 1 | log) * (($x / 3) | cos))
            - (($x * $x + 1) | sqrt)
          )
        | fabs
      ]
    | add
  ]
| add
