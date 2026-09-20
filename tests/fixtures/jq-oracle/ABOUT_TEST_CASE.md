# About test case

- For the test script, see `tests/integration/jq_oracle.rs`
- For test case with a name 'A', there must be a jq script file `A.jq`.
  - If the file name started with '_', it'll be ignored. (Used for shared module/data file)
- If there is some input jsons, `A.in.jsonl` will be used. If the file does not exists, it'll just put EOF
- At the top of script, you may put `#! <OPTIONS>` for the test. for example, if your test need CLI option `--stream --raw-input --arg x 20`,

```jq my_test.jq
#! --stream --raw-input --arg x 20
. | length
```
