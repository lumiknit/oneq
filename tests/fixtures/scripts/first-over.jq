# Return the first array element greater than 10, breaking out of a
# foreach loop early via label/break.

label $out |
foreach .[] as $x (0;
  . + 1;
  if $x > 10 then $x, break $out else empty end
)
