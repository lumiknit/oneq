# Test public native and jq-language signatures, not a build-dependent count.
builtins | {
  unique: (length == (unique | length)),
  native: (contains(["length/0", "type/0", "tonumber/0"])),
  prelude: (contains(["map/1", "select/1", "sort_by/1", "walk/1"]))
}
