# Integration scenario: apply a stream of config-patch records onto
# a shared JSON-imported default config, deep-merging objects and
# arrays, rejecting patches with a non-positive timeout, and
# rendering both a JSON and a shell-exportable view of the result.
#
# Exercises together: ?// pattern matching (patch may or may not be
# wrapped under "overrides"), import "<module>" + import "<json>" as
# $NAME, multiple defs with closures (positive_or_error($field)),
# try/catch + error(), math functions (ceil), and format strings
# (@sh, @json).

import "_mod_merge" as merge;
import "_data_defaults" as $defaults;

def base: $defaults::defaults[0];

def normalize_patch:
  . as {overrides: $o, env: $e} ?// {overrides: $o} ?// $o
  | {patch: $o, env: ($e // "dev")};

def apply($cfg):
  normalize_patch as {$patch, $env}
  | (try ($patch.timeout_ms // $cfg.timeout_ms | merge::positive_or_error("timeout_ms")) catch null) as $checked_timeout
  | if $checked_timeout == null then
      {env: $env, error: (try ($patch.timeout_ms | merge::positive_or_error("timeout_ms")) catch .)}
    else
      merge::deep_merge($cfg; $patch) as $merged
      | {
          env: $env,
          merged: $merged,
          mem_gb: ($merged.limits.mem_mb / 1024 | ceil),
          shell_env: ("TIMEOUT_MS=\($merged.timeout_ms) RETRIES=\($merged.retries)" | @sh),
          json_blob: ($merged | @json),
        }
    end;

[., inputs]
| map(apply(base))
