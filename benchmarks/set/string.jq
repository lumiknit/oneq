. as $n
| [
    range($n)
    | . as $i
    | "  Item-\($i)_ID:\(($i * 37) % 1000)-tag_\($i % 7)  "
    | ascii_downcase
    | gsub("^\\s+|\\s+$"; "")
    | gsub("[_:]"; "-")
    | capture("^item-(?<idx>\\d+)-id-(?<num>\\d+)-tag-(?<tag>\\d+)$") as $c
    | ($c.idx + "|" + $c.num + "|" + $c.tag)
    | split("|")
    | map(ascii_upcase)
    | join("_")
  ]
| add
