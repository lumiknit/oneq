{
  test: ("abc123" | test("[a-z]+[0-9]+")),
  test_flags: ("ABC" | test("abc"; "i")),
  match: ("abc123" | match("([a-z]+)([0-9]+)")),
  match_flags: ("ABC" | match("abc"; "i")),
  capture: ("abc-123" | capture("(?<a>[a-z]+)-(?<n>[0-9]+)")),
  capture_flags: ("ABC" | capture("(?<x>abc)"; "i")),
  scan: ("a1 b2" | [scan("[a-z][0-9]")]),
  splits: ("a,b;c" | [splits("[,;]")]),
  sub: ("abcabc" | sub("a"; "X")),
  gsub: ("abcabc" | gsub("a"; "X"))
}
