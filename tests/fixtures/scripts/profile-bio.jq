# Build a short bio line from a profile, tolerating a missing name
# via optional-postfix + alt ("//"), and taking the first two tags
# via a slice. Exercises "as" binding + string interpolation, quoted keys,
# negative/open-ended slices, and optional bracket indexing.

(.user.name? // "anonymous") as $name |
{
  "display name": $name,
  bio: "\($name) (\(.tags[-2:]? | join(", ")))",
  last_tag: .tags[-1]?
}
