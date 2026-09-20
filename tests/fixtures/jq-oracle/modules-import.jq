# Module import: uses _mod_util.jq via `import`, exercising functions
# defined there (double, add_all, greet).
import "_mod_util" as util;
{
  doubled: (.n | util::double),
  total: util::add_all(.nums),
  greeting: util::greet(.name)
}
