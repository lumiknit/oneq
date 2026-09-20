# Support module for user-pipeline.jq, pulled in with `include` (so
# its defs land directly in the includer's namespace, unprefixed).
def clamp($lo; $hi):
  if . < $lo then $lo elif . > $hi then $hi else . end;

def score_band:
  if . >= 90 then "A"
  elif . >= 75 then "B"
  elif . >= 50 then "C"
  else "D"
  end;

# closure: builds a checker bound over the passed-in minimum age
def age_checker($min_age):
  . as $age | if $age >= $min_age then $age else error("age below minimum \($min_age)") end;
