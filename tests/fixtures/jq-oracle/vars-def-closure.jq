# Variable bindings (as), array/object destructuring, function
# definitions, recursion, and closures.
def add_n($x): . + $x;
def make_adder($x): def inner: . + $x; inner;
def fact: if . <= 1 then 1 else . as $n | (. - 1 | fact) * $n end;
{
  as_simple: (.n as $x | $x * 2),
  destructure_arr: (.pair as [$a, $b] | {a: $a, b: $b}),
  destructure_obj: (.obj as {a: $a, b: $b} | {a: $a, b: $b}),
  def_call: (.n | add_n(100)),
  closure_call: (.n | make_adder(7)),
  recursive_fact: ((.n // 0 | if . < 0 then 0 elif . > 10 then 10 else . end) | fact)
}
