#! -n
# Keep streamed outputs and propagate the final alternative's uncaught error.
def unwrap: . as {value: $v} ?// $v | $v;
{} | unwrap | (., error("exhausted"))
