# Integration scenario: parse heterogeneous log-line records (two
# different schemas plus a sparse/malformed one), normalize them with
# the destructuring alternative operator (?//), classify latency with
# imported JSON thresholds, aggregate per-level stats with an
# imported math module, guard bad "level" values with try/catch +
# error(), and render the final report partly as @csv/@base64 text.
#
# Exercises together: ?// pattern matching, import "<module>",
# import "<json>" as $NAME, multiple defs with closures, try/catch,
# error(), math functions (sqrt/pow via the module, floor), and
# format strings (@csv, @base64).

import "_mod_stats" as stats;
import "_data_thresholds" as $cfg;

def known_levels: ["info", "warn", "error"];
def thresholds: $cfg::cfg[0];

def classify($ms):
  if $ms > thresholds.error_ms then "error"
  elif $ms > thresholds.warn_ms then "warn"
  else "ok"
  end;

def check_level($lvl):
  if (known_levels | index($lvl)) != null then $lvl
  else error("unknown level: \($lvl)")
  end;

def normalize:
  . as {level: $level, msg: $msg, duration_ms: $ms, user: {name: $name}}
    ?// {level: $level, message: $msg, elapsed: $ms, user: [$uid, $name]}
    ?// {level: $level}
  | {
      level: (try check_level($level // "warn") catch "warn"),
      msg: ($msg // "n/a"),
      ms: ($ms // 0),
      name: ($name // "unknown"),
    };

def summarize($entries):
  ($entries | map(.ms)) as $durations
  | {
      count: ($entries | length),
      mean_ms: (stats::mean($durations) | stats::round2),
      stddev_ms: (stats::stddev($durations) | stats::round2),
      worst_class: (($durations | max // 0) | classify(.)),
      p_slow_pct: ((([$entries[] | select(.ms > thresholds.warn_ms)] | length) * 100 / ($entries | length)) | floor),
    };

[., inputs]
| map(normalize)
| group_by(.level)
| map({level: .[0].level, report: summarize(.)})
| sort_by(.level)
| {
    by_level: .,
    csv_summary: [.[] | [.level, .report.count, .report.mean_ms, .report.worst_class]] | map(@csv) | join("\n"),
    encoded_levels: (map(.level) | join(",") | @base64),
  }
