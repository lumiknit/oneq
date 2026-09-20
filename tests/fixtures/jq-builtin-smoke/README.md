# jq builtin smoke suite

Run pure tests:

    for f in 0[1-9]_*.jq; do jq -n -f "$f" >/dev/null || exit 1; done

Special side-effect test:

    jq -n -f 10_special.jq

Notes:
- Intended as broad public-builtin smoke coverage, not a proof that every `builtins`
  name/arity/internal helper has been invoked.
- `input` / `inputs` require an actual input source and are not part of the pure
  null-input suite.
- `halt` / `halt_error` intentionally aren't included because they terminate jq.
- Some libc math/date builtins can vary by platform/build; `builtin_smoke_matches_jq`
  compares numbers with a small relative tolerance to absorb last-ULP differences.
- `11_builtins_jq.jq` checks unique public signatures from both native and
  jq-language builtins. The total count depends on each implementation's features.
- When jq reports that `exp10` was unavailable at build time, the Rust harness
  uses jq's equivalent `pow(10; .)` as the numeric reference for that field.
