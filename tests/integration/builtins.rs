use crate::common::{Report, list_fixtures, run_jq, run_oneq};
use oneq::data::Value;

/// Numbers only ever drift by the last ULP or so between libm implementations;
/// a relative tolerance well above that (but far below "actually wrong") lets
/// those through without masking a real correctness bug.
const NUMERIC_TOLERANCE: f64 = 1e-9;

fn approx_number(a: f64, b: f64) -> bool {
    if a == b {
        return true;
    }
    let scale = a.abs().max(b.abs()).max(1.0);
    ((a - b).abs() / scale) < NUMERIC_TOLERANCE
}

/// Structural equality with numeric tolerance, and object keys compared as an
/// unordered set (order already matched if the byte comparison would have).
fn approx_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| approx_equal(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).is_some_and(|bv| approx_equal(v, bv)))
        }
        (a, b) => match (a.as_number(), b.as_number()) {
            (Some(a), Some(b)) => approx_number(a, b),
            _ => false,
        },
    }
}

/// Runs every `.jq` fixture under `tests/fixtures/jq-builtin-smoke/`
/// as `jq -n -f <file>` (per the suite's own README) and checks that
/// `1q` produces the same exit status and (numeric-tolerant) output.
#[test]
fn builtin_smoke_matches_jq() {
    let mut report = Report::new("jq-builtin-smoke");
    for path in list_fixtures("tests/fixtures/jq-builtin-smoke", "jq") {
        let path_str = path.to_str().unwrap();
        let args = ["-n", "-f", path_str];
        let mut expected = run_jq(&args, b"");
        // Some jq builds expose exp10 but cannot execute it. Keep testing 1q's
        // exp10 against the equivalent jq pow expression on those platforms.
        if String::from_utf8_lossy(&expected.stderr).contains("exp10/0 not found at build time") {
            let source = std::fs::read_to_string(&path)
                .unwrap()
                .replace("| exp10)", "| pow(10; .))");
            expected = run_jq(&["-n", &source], b"");
        }
        let actual = run_oneq(&args, b"");
        let result = if !expected.status.success() {
            Err(format!(
                "reference jq failed: {}",
                String::from_utf8_lossy(&expected.stderr)
            ))
        } else if actual.status.code() != expected.status.code() {
            Err(format!(
                "exit code mismatch: expected {:?}, got {:?} ({})",
                expected.status.code(),
                actual.status.code(),
                String::from_utf8_lossy(&actual.stderr)
            ))
        } else if actual.stdout == expected.stdout {
            Ok(())
        } else {
            match (
                oneq::data::parse_json_str(&String::from_utf8_lossy(&expected.stdout)),
                oneq::data::parse_json_str(&String::from_utf8_lossy(&actual.stdout)),
            ) {
                (Ok(expected_value), Ok(actual_value))
                    if approx_equal(&expected_value, &actual_value) =>
                {
                    Ok(())
                }
                _ => Err(format!(
                    "stdout mismatch\n--- jq ---\n{}\n--- 1q ---\n{}",
                    String::from_utf8_lossy(&expected.stdout),
                    String::from_utf8_lossy(&actual.stdout)
                )),
            }
        };
        report.record(path_str, result);
    }
    report.finish();
}
