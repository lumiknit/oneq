# Support module for config-merge.jq, pulled in with `import`.
# Recursive deep-merge: objects merge key-by-key, arrays concatenate
# and dedupe, anything else is replaced by the right-hand value.
def deep_merge(a; b):
  if (a | type) == "object" and (b | type) == "object" then
    reduce (b | keys_unsorted[]) as $k
      (a; .[$k] = (if (a[$k] != null) then deep_merge(a[$k]; b[$k]) else b[$k] end))
  elif (a | type) == "array" and (b | type) == "array" then
    (a + b) | unique
  else
    b
  end;

def positive_or_error($field):
  if . > 0 then . else error("\($field) must be positive, got \(.)") end;
