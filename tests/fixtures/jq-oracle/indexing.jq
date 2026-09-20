# Basic filters: identity, field access, optional access, array/object
# indexing and slicing.
{
  identity: .,
  foo_bar: .foo.bar,
  foo_bar_opt: .foo.bar?,
  bracket_foo: .["foo"],
  missing_opt: .nope?,
  first_of_arr: .arr[0],
  last_of_arr: .arr[-1],
  first_two: .list[0:2],
  slice_2_4: .arr[2:4],
  slice_head: .arr[:3],
  slice_tail: .arr[-2:],
  whole_slice: [.arr[]]
}
