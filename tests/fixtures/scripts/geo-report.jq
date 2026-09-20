# Declare this program as a module, pull in a helper library and an
# included snippet, then mix a namespaced call with a local funcdef.
# Exercises module_decl + import_decl_full (with search metadata) +
# include_decl + funcdef + object construction.

module {name: "geo-report"};

import "helpers" as helpers {search: "tests/fixtures/modules"};
include "shared";

def area($w; $h): $w * $h;

{area: area(.width; .height), doubled_width: helpers::double(.width)}
