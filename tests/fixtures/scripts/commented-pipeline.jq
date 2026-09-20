# ------------------------------------------------------------------
# Order pipeline
# ------------------------------------------------------------------
# Step 1: keep only paid orders.
# Step 2: compute a discounted total.
# Step 3: tag orders over the free-shipping threshold.
# ------------------------------------------------------------------

def discount($rate):
  # discount is applied to the raw total, not the tax
  . * (1 - $rate);

.[]
| select(.paid) # inline trailing comment after a pipe stage
| . as $o
# comment sitting between a bind and its body
| {
    id: $o.id,
    total: (
      $o.total
      | discount(0.1)
      # comment right before the branch that decides free shipping
    ),
    free_shipping: (
      $o.total > 50
      # comment immediately before "and"
      and $o.paid
    )
  }
