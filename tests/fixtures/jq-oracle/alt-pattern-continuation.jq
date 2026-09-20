#! -n
# An alternative remains retryable after its function returns. Already emitted
# values survive a later error; caught errors and empty do not trigger retries.
def normalize:
  . as {overrides: $o, env: $e} ?// {overrides: $o} ?// $o
  | {patch: $o, env: ($e // "dev")};
def unwrap: . as {value: $v} ?// $v | $v;
def require_value: if . == null then error("missing") else . end;
{
  caller: [ {timeout_ms: 500, retries: 5} | normalize | .patch | require_value ],
  nested_calls: [def relay: unwrap; {} | relay | require_value],
  partial_outputs: [{} | unwrap | (., require_value)],
  caught: [{} | unwrap | try require_value catch "caught"],
  suppressed: [{} | unwrap | require_value?],
  empty: [{} | unwrap | empty],
  lexical_try: [{} | (try unwrap catch "inner") | require_value],
  outer_try: [try ({} | unwrap | error("exhausted")) catch .],
  rethrow: [try ({} | unwrap | try require_value catch error("again")) catch .],
  cleared_bindings: [
    {a: 1} as {a: $a} ?// $b
    | if $b == null then error("retry") else [$a, $b] end
  ],
  source_stream: [({}, {value: 3}, [4]) | unwrap | require_value],
  nested_alternatives: [
    {} | unwrap as $outer
    | {} | unwrap as $inner
    | if $outer == null or $inner == null then error("retry")
      else [$outer, $inner] end
  ],
  collection_boundary: [try ({} | [unwrap] | .[0] | require_value) catch "outside"],
  sibling_branch: [{} | (unwrap | require_value), "sibling"],
  lexical_try_error: [
    try ({} | (try unwrap catch "inner") | error("outside")) catch .
  ]
}
