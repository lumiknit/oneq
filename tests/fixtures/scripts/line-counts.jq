# For each string, convert to length of line.
# The output must be json array with the length of each line.

if type == "string" then
  . | split("\n") | map(length)
else
  error("Input must be a string")
end
