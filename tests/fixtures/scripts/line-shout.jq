# For each raw text line, report its length and whether it's
# shouting (all upper-case). Meant to be run with `-R`. Exercises
# object construction with several builtins, comparison, if/then/else.

{
  text: .,
  length: length,
  shout: (. == ascii_upcase),
  custom_constants_with_long: [1, 2, 3]
} | if .length > 0 then . else "empty line" end
