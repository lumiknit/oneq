# Data format fixtures

- `catalog.yaml` / `catalog.expected.jsons`: five YAML documents, anchors and
  aliases, merge precedence, nested collections, multiline text, dates and large
  integers. The expected JSON stream contains one value for each YAML document.
- `build.toml` / `build.expected.jsons`: TOML 1.0 dotted keys, tables extended
  later in the document, nested arrays of tables, inline tables, date/time
  values, multiline strings and integer limits.

`tests/test_cli_stack.rs` compares assembled values against these independently
authored expectations, and stream events against JSON serialized in the source's
insertion order. It exercises both normal and slurped input.
