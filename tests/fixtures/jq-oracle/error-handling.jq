# error(msg), try/catch, optional operator (?), and input_line_number.
def check: if .kind == "bad" then error("bad kind: \(.value // "null")") else .value end;
{
  try_catch_msg: (try check catch .),
  optional_field: (.value.nested?),
  optional_index: (.value[0]?),
  caught_type_error: (try (.value | error) catch .),
  line_no_is_number: (input_line_number | type == "number")
}
