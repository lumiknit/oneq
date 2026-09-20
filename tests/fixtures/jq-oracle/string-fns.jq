# String function grab-bag: length, utf8bytelength, split/join,
# trim family, startswith/endswith, explode/implode, case conversion.
{
  length: (.s | length),
  utf8len: (.s | utf8bytelength),
  split_comma: (.s | split(",")),
  join_back: (.s | split(",") | join("-")),
  ltrim_hello: (.s | ltrimstr("Hello")),
  rtrim_bang: (.s | rtrimstr("!")),
  starts: (.s | startswith("Hello")),
  ends: (.s | endswith("!")),
  explode_implode: (.s | explode | implode),
  downcased: (.s | ascii_downcase),
  upcased: (.s | ascii_upcase),
  to_str: (.s | tostring),
  trimmed: (.s | trim),
  ltrimmed: (.s | ltrim),
  rtrimmed: (.s | rtrim)
}
