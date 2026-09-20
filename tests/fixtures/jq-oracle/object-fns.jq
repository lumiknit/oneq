# to_entries, from_entries, with_entries, has, in, contains, inside,
# keys / keys_unsorted, values.
{
  entries: (.o | to_entries),
  roundtrip: (.o | to_entries | from_entries),
  upcased_keys: (.o | with_entries(.key |= ascii_upcase)),
  has_a: (.o | has("a")),
  a_in_o: (.o as $obj | "a" | in($obj)),
  contains_check: (.o | contains({a:1}) ),
  inside_check: (.o as $obj | {a:1} | inside($obj)),
  keys_sorted: (.o | keys),
  keys_raw: (.o | keys_unsorted),
  vals: (.o | [.[]]),
  values_filter: ([.o.a, null, .o.b] | map(values))
}
