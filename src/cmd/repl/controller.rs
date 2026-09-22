//! REPL owns the materialized next-input stream; Session never replaces input.
use crate::{
    data::{self, Value},
    io::Input,
    jq::{
        CompileOptions, Session,
        vm::{InputMode, RunOutcome, VmEvent, host::InputHost},
    },
};

const OUTPUT_LIMIT: usize = 100_000;
const STEP_LIMIT: usize = 128 * 1024 * 1024;
const RESUME_QUANTUM: usize = 128 * 1024;

/// A REPL completion candidate: the text to insert (`replacement`) versus
/// the (possibly more descriptive, e.g. `map/1`) text shown in the list.
pub(super) struct CompletionWord {
    pub replacement: String,
    pub display: String,
}

/// Folds a sorted arity set into a compact label: contiguous runs become
/// `lo..hi`, e.g. `{1, 2, 3, 5}` -> `"1..3,5"`.
fn format_arities(arities: &std::collections::BTreeSet<usize>) -> String {
    let mut parts = Vec::new();
    let mut iter = arities.iter().copied().peekable();
    while let Some(start) = iter.next() {
        let mut end = start;
        while iter.peek() == Some(&(end + 1)) {
            end = iter.next().expect("peeked");
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}..{end}")
        });
    }
    parts.join(",")
}

pub(super) struct Controller {
    session: Session,
    input: Vec<Value>,
    scripts: Vec<String>,
}
impl Default for Controller {
    fn default() -> Self {
        Self {
            session: Session::new(),
            input: vec![Value::Null],
            scripts: vec![],
        }
    }
}
impl Controller {
    pub fn load(&mut self, source: &str, format: data::DataFormat) -> Result<(), String> {
        let parser =
            data::AnyParser::new(format, Input::new_str(source)).map_err(|e| e.to_string())?;
        let input = data::ValueBuilder::new(parser, data::builder::StreamOption::default())
            .map(|v| v.map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        self.input = input;
        Ok(())
    }
    pub fn dump(&self) -> String {
        self.scripts.join("\n\n")
    }
    /// Function and variable names currently in scope, for REPL completion.
    /// Variables are prefixed with `$` so they complete against the same
    /// token a user would actually type. Each function name is one
    /// candidate (its arities are folded into the display label, e.g.
    /// `test/1..2`) so a name completes immediately once it's unambiguous,
    /// even when it's defined at more than one arity.
    ///
    /// Includes both `def`-based functions (builtin.jq prelude, and
    /// whatever the user has defined in this session) and natively
    /// implemented builtins (`length`, `keys`, ...), which never appear as
    /// `def`s and so aren't visible through `Session::function_names`.
    pub fn completion_words(&self) -> Vec<CompletionWord> {
        let mut arities: std::collections::BTreeMap<&str, std::collections::BTreeSet<usize>> =
            std::collections::BTreeMap::new();
        for spec in crate::jq::builtins::registry() {
            arities
                .entry(spec.name)
                .or_default()
                .insert(spec.params.len());
        }
        for (name, arity) in self.session.function_names() {
            arities.entry(name).or_default().insert(arity);
        }
        let mut words: Vec<CompletionWord> = arities
            .into_iter()
            .map(|(name, arities)| CompletionWord {
                display: format!("{name}/{}", format_arities(&arities)),
                replacement: name.to_owned(),
            })
            .collect();
        words.extend(self.session.variable_names().map(|name| CompletionWord {
            replacement: format!("${name}"),
            display: format!("${name}"),
        }));
        words
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn evaluate(&mut self, source: &str) -> Result<&[Value], String> {
        let checkpoint = self.session.checkpoint();
        let entry = self
            .session
            .append(
                source,
                CompileOptions {
                    path: "<repl>".into(),
                    repl: true,
                    ..CompileOptions::default()
                },
            )
            .map_err(|e| e.to_string())?;
        let definition_only = self
            .session
            .is_definition(entry)
            .map_err(|e| e.to_string())?;
        let mut host = InputHost::new(self.input.iter().cloned().map(Ok));
        let result = (|| {
            let mut run = self
                .session
                .run(entry, &mut host, InputMode::Host)
                .map_err(|e| e.to_string())?;
            let mut output = Vec::new();
            let mut budget = STEP_LIMIT;
            while budget > 0 {
                let quantum = budget.min(RESUME_QUANTUM);
                budget -= quantum;
                match run.resume(quantum) {
                    VmEvent::Output(value) => {
                        if output.len() == OUTPUT_LIMIT {
                            run.cancel();
                            return Err(
                                "REPL output limit exceeded; input and state preserved".into()
                            );
                        }
                        output.push(value);
                    }
                    VmEvent::Done if run.outcome() == Some(&RunOutcome::Complete) => {
                        return Ok(output);
                    }
                    VmEvent::Error(error) => return Err(error.user_message()),
                    VmEvent::Halt { code, .. } => {
                        return Err(format!("halt ({code}); input and state preserved"));
                    }
                    VmEvent::Done => return Err("execution cancelled".into()),
                    VmEvent::Suspended => {}
                }
            }
            run.cancel();
            Err("REPL execution budget exceeded; input and state preserved".into())
        })();
        match result {
            Ok(output) => {
                self.scripts.push(source.to_owned());
                if !definition_only {
                    self.input = output;
                }
                Ok(&self.input)
            }
            Err(error) => {
                self.session
                    .rollback(checkpoint)
                    .map_err(|e| e.to_string())?;
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn strings(values: &[Value]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }
    #[test]
    fn output_becomes_next_input_and_empty_stays_empty() {
        let mut repl = Controller::default();
        assert_eq!(strings(repl.evaluate("1,2").unwrap()), ["1", "2"]);
        assert_eq!(
            strings(repl.evaluate("type").unwrap()),
            ["\"number\"", "\"number\""]
        );
        assert!(repl.evaluate("empty").unwrap().is_empty());
        assert!(repl.evaluate("42").unwrap().is_empty());
        repl.reset();
        assert_eq!(strings(repl.evaluate(".").unwrap()), ["null"]);
    }
    #[test]
    fn load_and_execution_failure_preserve_input() {
        let mut repl = Controller::default();
        repl.load("1\n2", data::DataFormat::Json).unwrap();
        assert!(repl.load("3\n[", data::DataFormat::Json).is_err());
        assert!(repl.evaluate("1,error(\"bad\")").is_err());
        assert_eq!(strings(repl.evaluate(".").unwrap()), ["1", "2"]);
        assert!(repl.evaluate("halt").is_err());
        assert!(repl.evaluate("(").is_err());
        assert_eq!(strings(repl.evaluate(".").unwrap()), ["1", "2"]);
    }
}
