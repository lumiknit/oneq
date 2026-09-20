label $out | .[] | if .>2 then break $out else . end
