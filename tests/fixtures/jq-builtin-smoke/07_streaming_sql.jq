{
  tostream: ({a:[1,2]} | [tostream]),
  fromstream: ([["a"],1], [["b"],2] | fromstream(.)),
  truncate_stream: ([0,[1,2]] | [truncate_stream(1)]),
  INDEX1: ([{id:"a",v:1},{id:"b",v:2}] | INDEX(.id)),
  INDEX2: (INDEX([{id:"a",v:1},{id:"b",v:2}][]; .id)),
  IN1: ("b" | IN("a","b","c")),
  IN2: (IN("b"; "a","b","c")),
  JOIN2: (
    [{id:"a",v:1}] | INDEX(.id) as $idx |
    {id:"a"} | JOIN($idx; .id)
  )
}
