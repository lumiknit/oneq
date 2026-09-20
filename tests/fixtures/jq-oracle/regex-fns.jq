# Regex-related builtins: test, match, capture, scan, split(re;flags),
# splits, sub, gsub.
{
  test_digit: (.s | test("[0-9]+")),
  match_first: (.s | [match("[0-9]+")] | map({string, offset})),
  capture_date: (.s | capture("(?<y>[0-9]{4})-(?<m>[0-9]{2})-(?<d>[0-9]{2})")? // null),
  scan_words: (.s | [scan("[A-Za-z]+")]),
  split_re: (.s | split(" +"; null)),
  splits_re: [.s | splits("[0-9]+")],
  sub_first: (.s | sub("[0-9]+"; "#")),
  gsub_all: (.s | gsub("[0-9]+"; "#"))
}
