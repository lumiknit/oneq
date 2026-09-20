# Date/time helpers: gmtime, mktime, strftime, strptime.
# now() itself is non-deterministic, so we only check its shape, not value.
(.t // (.s | strptime("%Y-%m-%dT%H:%M:%SZ") | mktime)) as $epoch
| {
  epoch: $epoch,
  gmtime_arr: ($epoch | gmtime | map(if type == "number" then (. | floor) else . end)),
  roundtrip: ($epoch | gmtime | mktime),
  formatted: ($epoch | gmtime | strftime("%Y-%m-%dT%H:%M:%SZ")),
  parsed_back: ($epoch | gmtime | strftime("%Y-%m-%dT%H:%M:%SZ") | strptime("%Y-%m-%dT%H:%M:%SZ") | mktime),
  now_is_number: (now | type == "number")
}
