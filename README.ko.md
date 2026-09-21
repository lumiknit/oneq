# oneq

`1q` (원큐) 는 [jq](https://jqlang.org/) 와 호환성을 보장하는 확장 CLI & WASM 도구입니다.

## Quick Start

- Web Playground: https://lumiknit.github.io/apps/1q/
- Install CLI

```sh
cargo install --git https://github.com/lumiknit/oneq
```

## Features

- `jq` 대신 `1q` 를 그대로 쓸 수 있는 drop-in replacement
  - Built-ins, CLI Options 등 `jq` cli 에서 문제 없이 동작하는 기능은 가능한 동일하게 사용할 수 있도록 함.
  - `--stream`, `--stream-errors`, `--seq` 등 옵션 지원.
  - 현재는 `jq 1.8.2` 를 기반으로 구현이 되어 있고, 추후 `jq` 쪽 변경사항이 있을 경우 `jq` 에 우선적으로 기능을 맞춤.
- 각종 확장 기능들
  - `--from`, `--to`: `json` 외에 JSON5, YAML, TOML, Python Literal, Environment, CSV, XML 등을 지정 가능
  - `--from` / `--to` 등에 대해 `jq` 자체를 지정 가능, 또는 `--fmt`: AST 를 편집하거나 `jq script` 자체를 포매팅할 수 있음.
  - `--doc` 으로 상세한 builtin 문서 조회 가능
  - `--repl` repl 모드

### Differences with Original JQ

- 정규식 엔진으로 oniguruma 대신 Rust 표준 regex 사용함.
- JSON 등 데이터 처리 중 오류 발생 시 Buffering 으로 인해 오류 위치가 약간 부정확할 수 있음.

## Usage

### CLI

- 자세한 사용법은 `1q --help` 를 사용해주세요.
- 기본적으로 `jq` 의 사용법은 가능한 그대로 제공하기 때문에 https://jqlang.org/manual/ 도 참고해주세요.

### External Format

1q 에서는 JSON 외에도 여러가지 다른 포맷을 입력받거나 출력할 수 있습니다.
입출력 시 `--from` (`-F`) 와 `--to` (`-T`) 로 입력을 받을 수 있으며 아래와 같은 포맷이 지원됩니다.

| Name | Description | Parser? | Serializer? (Support compact?) | Doc. Separator |
|-|-|-|-|-|
| json | (Default) JSON (RFC8259) | Y | Y (Y) | *any (2)* |
| cbor | CBOR (RFC 8949) | Y | Y (binary; output options ignored) | none |
| json5 | JSON5 | Y | alias of `json` | *any (2)* |
| j | Loose JSON (Allow most json-like structures) | Y | alias of `json` | *any (2)* |
| pylit | Python literal (None, True, False, etc.) | Y | Y | *any (2)* |
| yaml | YAML 1.2 | Y | Y (Y) | `\n---\n` |
| toml | TOML 1.0 | Y | Y (N) | `\n+++\n` |
| csv | CSV (RFC 4180) | Y | Y (N) | `\n\n` |
| csvh | CSV with header | Y | Y (N) | `\n\n` |
| tsv | TSV | Y | Y (N) | `\n\n` |
| tsvh | TSV with header | Y | Y (N) | `\n\n` |
| env | Takes `KEY=VALUE` pairs form. (allow dotenv, export env) Output is shell-compatible. | Y | Y (N) | none |
| exportenv | `export KEY=VALUE` | N | Y (N) | none |
| xml | XML | Y | **P(1)** (Y) | *any (2)* |
| jq | jq Syntax Tree | Y | **P(1)** (Y) | none |
| raw | Same as '-R' | Y | Y | none |
| rawslurp | Same as '-R --slurp' | Y | Y | none |

\*\* (1) `Serializer: P` : 필터 처리 후 결과가 포맷과 맞지 않으면 출력 실패할 수 있음.

\*\* (2) `any`: JSON, XML 등은 구분이 없어도 데이터의 끝을 알 수 있어서, 그 부분를 문서의 끝으로 취급, 출력 시에는 `\n` 이 추가될 수 있음.

포맷 이름은 대소문자를 구분하지 않으며 ascii alphanumeric 외 글자는 무시됩니다. 예시로 `json`, `Json`, `J_S_O_N` 모두 허용됩니다.

포맷과 관련된 주의사항이 있습니다.

- JSON 을 제외한 다른 포맷은 입력 시 JSON 에서 다루는 값 (null, bool, number, string, array, object) 를 제외한 데이터는 전부 유실됩니다.
  - 주석, datetime 등 지원되지 않는 값은 전부 누락됩니다.
  - TOML 등에서의 Section 순서, YAML 의 Anchor 등도 누락됩니다.
- JSON 을 제외한 포맷에서는 document separator 가 종류별로 다릅니다.
- `xml`, `jq` 등은 json 과 일대일 대응이 되지 않기 때문에 필터의 결과가 스키마와 다른 경우에는 출력에 실패할 수 있습니다.
- `toml` 의 경우에는 `null` 이 없기 때문에 null 필드는 없는 것으로 간주하고 생략됩니다. 배열 등에서는 개수가 달라질 수 있습니다.
- 대부분은 `--stream` 옵션이 있는 경우 스트리밍 파싱을 시도하지만 일부 데이터 형식 (e.g. csv with headers, jq 등) 에서는 파일 일부 또는 전체를 미리 읽는 과정이 있을 수 있습니다.
- `jq` 에서 지원하는 `--raw-input`, `--raw-output`, `--join-output` 등의 옵션이 주어지면 from/to 에서 지정되는 옵션을 덮어쓰게 됩니다. 오류 없이 실행되므로 주의해주세요.

### JSON-like

JSON, JSON5, Python Literal, Loose 등은 전부 JSON 과 비슷한 방식으로 파싱을 지원하는 계열입니다. trailing comma 라든가 keyword (null, None, True, false 등) 에 구체적인 차이가 있는 정도입니다.

다만 `jq` 에서는 조금 더 관용적으로 데이터를 받아들입니다.

- `Infinity`, `NaN` 등은 strict JSON 에서도 허용.
  - 단, strict JSON 출력 시에는 `jq` 와 같은 동작으로, `NaN` 은 `null`, `Infinity` 는 `f64::max` 등으로 치환됩니다.
- 키워드의 대소문자는 구분하지 않음.
- 중복된 field 허용 (정확히는 여러번 같은 field 가 나오면 덮어씀)

### Output Format Style

1q 에서는 jq 에서도 지원되는 `--compact-output` (`-c`) 가 지원됩니다. 이외에도 추가적으로 `--inline-output` 이 있으며 compact 와 비슷하게 한 줄 출력을 시도하지만 콤마 뒤에 공백을 넣는 등 조금 더 보기 좋은 형식으로 출력합니다.

대부분의 경우에는 위 옵션을 지원하지만 `csv` 등 일부 한줄로 압축이 힘든 경우에는 기본 출력 (pretty print) 와 compact 등이 동일하게 나올 수 있습니다.

마찬가지로 색상 출력 `--color-output` (`-C`), `--monochrome-output` (`-M`) 옵션이 사용 가능한데, 출력이 tty 인 경우에는 기본적으로 color output 이 나오게 됩니다.

### Stream Option

`--stream` 옵션을 사용하게 되면 JSON Value 대신 각 path 별 값을 입력하게 되며,
항목 내에서 값을 입력할 때마다 경로와 값이 필터로 전달됩니다.

JSON, YAML 등 대부분의 경우에는 한번 방문한 path 를 추가 방문하지 않지만,
TOML 의 경우에는 섹션을 재방문 하는 경우가 발생할 수 있습니다.
이때도 stream 의 규칙에 맞게 출력을 하지만 엣지케이스가 있다는 점은 참고해주세요.

예를 들어서, `1q --stream -F toml -c .` 의 경우 각 줄 입력에 대해서 아래와 같이 filter 에 값이 전달됩니다.

```toml
[a.b]
t = 42
#=> [["a","b","t"],42]

[c]
#=> [["a","b","t"]]
#=> [["a","b"]]

z = 5
#=> [["c","z"],5]

# EOF
#=> [["c","z"]]
#=> [["c"]]
```

1q 내부적으로는 기본적으로  parser 는 위와 같은 stream 으로 처리를 하며, 아래와 같은 규칙으로 JSON 변수로 합칩니다.

- `[PATH, value]` 가 있으면 해당 PATH 까지 진입 후 값을 삽입. 만약 path 중간 object/array 가 없는 경우에는 생성
- `value` 에는 아무 값이나 허용. (jq 에서는 null/bool/number/string 과 빈 object/array 만 허용됨.)
- `[PATH]` 의 경우 `PATH` 길이가 2 이상인 경우에는 아무런 처리를 하지 않음.
- 길이가 1이나 0인 경우에는 (어떤 path 든 상관 없이) document 종료로 취급하고 하나의 JSON Value 로 내보냄.

### Format JQ Script

1q 에는 `jq` 스크립트 자체를 입출력에 사용할 수 있습니다.
또한 compact, oneline, pretty print, color output 등이 모두 지원되어서 jq 스크립트 포매터로 사용할 수 있습니다.

`--fmt` 옵션은 `-F jq -T jq .` (in/out 모두 `jq` 로 지정하고 필터를 by-pass 로 지정) 의 alias 입니다.

```sh
# Format 'my_script.jq' and pretty print & color to the stdout
cat 'my_script.jq' | 1q -F jq -T jq .

# Shorthand: --fmt. Equivalent to `-F jq -T jq .`
cat 'my_script.jq' | 1q --fmt
```

### REPL

`1q --repl` 을 사용하면 REPL 모드로 사용할 수 있습니다. 인자 없이 인터랙티브 터미널에서 `1q`를 실행해도 REPL로 시작합니다.
일반적인 `1q <FILTER>` 가 고정된 필터로 입력을 계속 stdin 으로 넣을 수 있다면,
REPL 에서는 입력에 대해 새로운 필터를 추가하고, 그 출력에 대해 새로운 필터를 실행하는 것이 가능합니다.

`:` 으로 시작하는 명령을 사용할 수 있으며, 종료를 원하면 SIGINT (`^c`) 를 두번 보내서 종료할 수 있습니다. Rustyline 의 동작을 참고해주세요.

- `:help` or `:h` Show help message
- `:doc` or `:d` Show built-in function references
- `:grammar` or `:g` Show grammar references
- `:exit` or `:q` Exit REPL
- `:load <PATH>` 또는 `:l`/`:file`/`:f` 파일에서 입력을 읽습니다. 지원되는 확장자는 입력 포맷으로 사용됩니다.
- `:to <FORMAT>` 출력 serializer를 지정합니다. 예: `:to yaml`
- `:dump [PATH]` 지금까지 실행한 jq 스크립트를 출력하거나 파일에 저장합니다.

REPL 시 일반 1q 필터 와 아래 같은 차이점이 있습니다:

- 여타 REPL 과 마찬가지로 실행할 때마다 상태가 누적될 수 있습니다.
- `as $t` 라든가 `def` 등으로 함수/변수를 전역 스코프에서 정의하면 다음 필터 입력에서도 사용할 수 있습니다. **다만, 다음 입력에서는 가장 마지막에 설정된 as, def 값으로 나오기 때문에 단순히 pipe 로 연결한 것과는 결과가 다를 수 있습니다.

```jq
1,2,3 | . as $a | . | $a + .

# When `. | $a + .` runs for each 1, 2, 3
#=> 2
#=> 4
#=> 6
```

```jq-repl
1q> 1, 2, 3 | . as $a | .

# Since initial input is null, this prints null, and $a is 3
#=> 1
#=> 2
#=> 3

1q> $a + .
#=> 4
#=> 5
#=> 6
```

---

## Project

### Status

| Level | Name | Description |
|-|-|-|
| 0 | Planned | 계획, 미구현 |
| 1 | Experimental | 구현중이지만 불안정, 사용 비권고 |
| 2 | Alpha | 구현은 안정, 테스트 중인 기능, 사용 비권고 |
| 3 | Beta | 구현은 안정, 테스트 진행 중, 대부분의 데이터/입력에 대해 동작하지만 완전 검증되지는 않음 |
| 4 | Stable | 기능 동작 보장 |

| Feature | Level | Details |
|-|-|-|
| Data: JSON | 3 Beta | Parsing & Serialization |
| Data: jq | 1 Experimental |  Parsing & Serialization |
| Data: Others | 2 Alpha | Parsing & Serialization|
| Compiler: Lowering | 2 Alpha | . |
| Optimizer: IR-level | 0 Planned | . |
| Optimizer: CalcBlock | 0 Planned | . |
| VM: Naive Filter | 3 Beta | `.` 등 일부 자명한 필드, 단순 빌트인 호출 등 |
| VM: Full VM | 2 Alpha | . |
| VM: REPL VM | 1 Experimental | . |
| jq cli option compatibility | 3 Beta | . |
| WebUI:  | 2 Alpha | . |

### Test

- Original jq test (https://github.com/jqlang/jq/blob/master/tests/jq.test)
  - 531 / 550 Pass

### Benchmark

`/benchmarks` 폴더에서 관리합니다. `go run /benchmarks/compare.go` 로 비교할 수 있습니다.

현재는 original `jq` 와 거의 비슷한 (테스트 케이스에 따라 0.9~1.2배 범위 정도 속도의) 성능을 보여주고 있습니다.
