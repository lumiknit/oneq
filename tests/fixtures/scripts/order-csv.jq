# Turn a list of [id, qty, price] triples into CSV rows with a
# computed subtotal. Exercises a local funcdef, array-pattern
# destructuring, and the "@csv" format filter.

def subtotal($qty; $price): $qty * $price;

.[] |
. as [$id, $qty, $price] |
[$id, $qty, $price, subtotal($qty; $price)] | @csv
