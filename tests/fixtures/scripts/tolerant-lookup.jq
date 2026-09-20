# Chained "//" defaults on optional-postfix lookups, plus a local
# try/catch to guard a parse that might fail - the usual way to make
# a filter tolerant of missing or malformed fields.

def safe_num: try tonumber catch null;

(.a.b? // .c? // "none") as $v |
{value: $v, risky: (.raw | safe_num)}
