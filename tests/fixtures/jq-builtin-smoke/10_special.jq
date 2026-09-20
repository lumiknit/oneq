# Side-effect/error/meta smoke tests that still work with `jq -n -f`.
# `input` and `inputs` are intentionally excluded: they need an input source
# even when jq itself is invoked with -n.
{
  error0: (try ("boom" | error) catch .),
  error1: (try error("boom") catch .),
  debug: (1 | debug),
  debug_msg: (1 | debug("value=\(.)")),
  stderr: ("stderr-smoke" | stderr),
  input_filename: input_filename,
  input_line_number: input_line_number
}
