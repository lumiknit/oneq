# Arithmetic operators (+ - * / %) across numbers, strings, arrays,
# objects, and null, using try/catch to keep invalid combos safe.
{
  num_add: (.n1 + .n2),
  num_sub: (try (.n1 - .n2) catch "sub-error"),
  num_mul: (try (.n1 * .n2) catch "mul-error"),
  num_div: (try (.n1 / .n2) catch "div-error"),
  num_mod: (try (.n1 % .n2) catch "mod-error"),
  str_add: (.s1 + .s2),
  str_mul_int: (try (.s1 * 2) catch "str-mul-error"),
  arr_add: (.a1 + .a2),
  obj_add: (.o1 + .o2),
  obj_mul_deep: (try (.o1 * .o2) catch "objmul-error"),
  null_add: (null + .n2),
  null_arr_add: (null + .a2),
  add_with_null_rhs: (.n1 + null)
}
