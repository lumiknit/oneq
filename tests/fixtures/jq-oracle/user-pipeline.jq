# Integration scenario: validate & transform a mixed batch of user
# records (object form vs. positional array form) into a report,
# rejecting underage users without failing the whole batch.
#
# Exercises together: ?// pattern matching (object vs array shape),
# `include` (unprefixed shared defs with closures), // alternative
# operator, try/catch + error() for soft-rejection, math functions
# (pow/sqrt for a simple stddev, floor), and the @html format string.

include "_mod_validate";

def display_name:
  . as {$name, $nickname}
  | ($nickname // $name) | @html;

[., inputs]
| map(
    . as {name: $name, nickname: $nickname, age: $age, scores: $scores}
      ?// [$name, $age, $scores]
    | {
        display: ({name: $name, nickname: $nickname} | display_name),
        age_status: (try ($age | age_checker(18) | "ok") catch "rejected: \(.)"),
        clamped_age: ($age | clamp(0; 120)),
        avg_score: (($scores | add / length) as $avg | $avg),
        band: (($scores | add / length) | score_band),
        score_spread: (
          ($scores | add / length) as $mean
          | ($scores | map(pow(. - $mean; 2)) | add / length | sqrt)
          | (. * 100 | floor) / 100
        ),
      }
  )
| sort_by(.display)
