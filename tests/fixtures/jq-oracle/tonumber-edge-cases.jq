#! -n
[null, true, [], {}, "abc", "true"] | map(try tonumber catch .)
