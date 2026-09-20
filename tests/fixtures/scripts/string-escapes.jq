# A grab-bag of string escapes in one object literal: tab/newline/
# quote/backslash, a \u unicode escape, and interpolation mixed with
# literal escapes inside the same string. Also exercises a prefixed format
# string, a computed object key, and an arbitrary quoted bracket lookup.

{
  label: "Row:\t\"quoted\"\n(next line)\\ends-with-backslash",
  heart_literal: "❤",
  heart_escaped: "\u2764",
  mixed: "value=\(.n)\t<-\tsee \"tab\" above",
  json_fragment: @json "n=\(.n)",
  ("computed-\(.n)"): .n,
  arbitrary_key: .["not-an-identifier"]?
}
