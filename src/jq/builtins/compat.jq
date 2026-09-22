# jq-language additions absent from the unchanged upstream builtin.jq fixture.
def leaf_paths: paths(scalars);
def toboolean:
  if . == true or . == "true" then true
  elif . == false or . == "false" then false
  else error("\(type) (\(tojson)) cannot be parsed as a boolean") end;

# 1q originals
def randint(n): randint(0; n);
