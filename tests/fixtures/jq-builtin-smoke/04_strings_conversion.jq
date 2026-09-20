{
  tostring: (123 | tostring),
  tonumber: ("123.5" | tonumber),
  tojson: ({a:1} | tojson),
  fromjson: ("{\"a\":1}" | fromjson),
  ascii_downcase: ("AbC" | ascii_downcase),
  ascii_upcase: ("AbC" | ascii_upcase),
  startswith: ("abcdef" | startswith("abc")),
  endswith: ("abcdef" | endswith("def")),
  ltrimstr: ("foobar" | ltrimstr("foo")),
  rtrimstr: ("foobar" | rtrimstr("bar")),
  split: ("a,b,c" | split(",")),
  join: (["a",1,true,null] | join(",")),
  explode: ("Aé" | explode),
  implode: ([65,233] | implode)
}
