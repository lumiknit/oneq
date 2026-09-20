# Pipe, comma, parenthesized grouping, array/object constructors and
# shorthand object construction ({user, title}).
{
  comma_pair: [(.user, .title)],
  piped: (. | .age | (. + 1)),
  arr_ctor: [.user, .title, .age],
  obj_ctor: {u: .user, t: .title},
  shorthand: {user, title} + {age},
  nested_ctor: {info: {user, age}, tags: [.tags[]]},
  grouped: ((.age + 1) * 2)
}
