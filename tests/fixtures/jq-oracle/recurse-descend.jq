# Recursive descent (..), recurse, recurse(f), recurse(f; cond).
{
  all_numbers: [.. | numbers],
  all_scalars: [.. | scalars],
  recurse_default: [recurse | numbers],
  recurse_children: (if (.tree? // null) != null
      then [.tree | recurse(.children[]?) | .v? // empty]
      else [] end),
  recurse_cond: [1 | limit(5; recurse(. + 1; . < 100))]
}
