# @-format strings: base64/base64d, csv, tsv, html, uri, sh, json, text.
{
  b64_roundtrip: (.s | @base64 | @base64d),
  csv_row: (.row | @csv),
  tsv_row: (.row | @tsv),
  html_escaped: (.s | @html),
  uri_escaped: (.url | @uri),
  sh_quoted: (.s | @sh),
  json_str: (.row | @json),
  text_str: (.s | @text)
}
