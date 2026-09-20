# paths, leaf_paths, getpath, setpath, delpaths, path(EXPR)
{
  all_paths: [paths],
  leaf_paths: [paths(scalars)],
  path_of_a_b: [path(.a.b?)],
  getpath_a_b: getpath(["a","b"]),
  setpath_new: setpath(["a","z"]; 99),
  delpaths_a_b: delpaths([["a","b"]]),
  path_expr_select: [path(.. | select(type == "number"))]
}
