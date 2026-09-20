# Mixes a $-prefixed global ($ENV) with field access, and drains the
# rest of the input stream via the zero-arg `inputs` builtin to count
# how many more values follow the first one.

{home: $ENV.HOME, first: ., rest_count: ([inputs] | length)}
