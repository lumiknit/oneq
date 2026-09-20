# Recursive descent ("..") picks out every value in the input, then
# `numbers` filters to just the numeric leaves, and the whole thing
# is used as a path expression for update-assignment - doubling every
# number found anywhere in the structure, however deeply nested.

(.. | numbers) |= (. * 2)
