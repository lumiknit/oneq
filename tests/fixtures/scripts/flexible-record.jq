# "?//" tries several destructuring shapes for a single "as" binding
# in order, until one matches without erroring - handy for records
# whose shape isn't fully pinned down.

. as {name: $n} ?// [$n] ?// $n |
"name=\($n)"
