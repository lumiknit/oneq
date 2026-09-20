# Object/array merge (+, recursive *), del, type, tonumber/tostring,
# isnan/isinfinite/isnormal.
{
  shallow_merge: (.a + .b),
  deep_merge: (.a * .b),
  del_x: (.a | del(.x)),
  del_multi: (.a | del(.x, .y)),
  type_of_each: [.a, .b, .num_str, .big, null, true, [1]] | map(type),
  parsed_num: (.num_str | tonumber),
  back_to_str: (.num_str | tonumber | tostring),
  nan_check: ((0/0)? // "nan-error" | if . == "nan-error" then true else isnan end),
  infinite_check: (.big * 1e300 | isinfinite),
  normal_check: (.big | isnormal)
}
