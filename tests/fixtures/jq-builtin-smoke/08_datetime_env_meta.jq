{
  fromdateiso8601: ("2015-03-05T23:51:47Z" | fromdateiso8601),
  todateiso8601: (0 | todateiso8601),
  fromdate: ("2015-03-05T23:51:47Z" | fromdate),
  todate: (0 | todate),
  strptime: ("2015-03-05T23:51:47Z" | strptime("%Y-%m-%dT%H:%M:%SZ")),
  strftime: (0 | strftime("%Y-%m-%d")),
  gmtime: (0 | gmtime),
  mktime: ([1970,0,1,0,0,0,4,0] | mktime),
  now_is_number: (now | type),
  env_is_object: (env | type),
  loc: $__loc__
}
