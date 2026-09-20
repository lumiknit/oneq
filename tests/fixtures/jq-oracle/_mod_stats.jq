# Support module for log-report.jq (not a standalone test script).
# Small math helpers built from def + closures over an argument.
def mean(arr):
  if (arr | length) == 0 then null
  else (arr | add) / (arr | length)
  end;

def stddev(arr):
  (arr | length) as $n
  | if $n < 2 then 0
    else
      mean(arr) as $m
      | (mean(arr | map(pow(. - $m; 2))) | sqrt)
    end;

def round2:
  . as $x | (($x * 100) | round) / 100;
