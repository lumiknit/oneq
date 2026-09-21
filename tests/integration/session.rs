use oneq::{
    data::Value,
    jq::{
        CompileOptions, EntryId, Session, builtins,
        compiler::analyze::Cardinality,
        vm::{
            InputMode, JqError, RunOutcome, VmEvent,
            host::{Host, InputHost},
        },
    },
};
use std::{cell::Cell, rc::Rc};

fn number(n: i64) -> Value {
    Value::int(n as i64)
}
fn append(session: &mut Session, source: &str) -> EntryId {
    session.append(source, CompileOptions::default()).unwrap()
}
fn output(session: &mut Session, entry: EntryId, input: Vec<Value>) -> Vec<Value> {
    let mut host = InputHost::new(input.into_iter().map(Ok));
    let mut run = session.run(entry, &mut host, InputMode::Host).unwrap();
    let values = run.by_ref().collect::<Result<_, _>>().unwrap();
    assert_eq!(run.outcome(), Some(&RunOutcome::Complete));
    values
}
#[test]
fn identity_replays_and_append_does_not_recompile_old_code() {
    let mut session = Session::new();
    let first = append(&mut session, ". # comment is syntax trivia");
    let dump = session.dump(first).unwrap();
    let second = append(&mut session, "7");
    assert_ne!(first, second);
    assert_eq!(
        output(&mut session, first, vec![number(1), Value::Null]),
        vec![number(1), Value::Null]
    );
    assert_eq!(
        output(&mut session, first, vec![number(2)]),
        vec![number(2)]
    );
    assert_eq!(
        output(&mut session, second, vec![Value::Null]),
        vec![number(7)]
    );
    assert_eq!(session.dump(first).unwrap(), dump);
    assert!(!dump.contains("comment"));
}
#[test]
fn names_are_resolved_to_stable_ids_and_failed_append_is_atomic() {
    let mut session = Session::new();
    session.bind("x", number(1));
    let first = append(&mut session, "$x");
    assert!(session.append("($x |", CompileOptions::default()).is_err());
    assert!(
        session
            .append("$missing", CompileOptions::default())
            .is_err()
    );
    session.bind("x", number(2));
    let second = append(&mut session, "$x");
    assert_eq!(
        output(&mut session, first, vec![Value::Null]),
        vec![number(1)]
    );
    assert_eq!(
        output(&mut session, second, vec![Value::Null]),
        vec![number(2)]
    );
}
#[test]
fn rollback_invalidates_entries_without_reusing_ids_and_restores_names() {
    let mut session = Session::new();
    session.bind("x", number(1));
    let first = append(&mut session, "$x");
    let checkpoint = session.checkpoint();
    session.bind("x", number(2));
    let cancelled = append(&mut session, "$x");
    session.rollback(checkpoint).unwrap();
    let next = append(&mut session, "$x");
    assert_ne!(cancelled, next);
    assert!(session.dump(cancelled).is_err());
    let mut host = InputHost::new(std::iter::empty());
    assert!(matches!(
        session.run(cancelled, &mut host, InputMode::Null),
        Err(JqError::InvalidEntry)
    ));
    assert_eq!(
        output(&mut session, first, vec![Value::Null]),
        vec![number(1)]
    );
    assert_eq!(
        output(&mut session, next, vec![Value::Null]),
        vec![number(1)]
    );
    let mut foreign = Session::new();
    let foreign_entry = append(&mut foreign, ".");
    assert!(matches!(
        session.run(foreign_entry, &mut host, InputMode::Null),
        Err(JqError::InvalidEntry)
    ));
    assert!(session.rollback(foreign.checkpoint()).is_err());
}
#[test]
fn depth_first_choices_restore_original_input_and_operands() {
    let mut session = Session::new();
    let entry = append(&mut session, "((1,2)|(3,4)),.,(empty,5)");
    assert_eq!(
        output(&mut session, entry, vec![number(9)]),
        vec![
            number(3),
            number(4),
            number(3),
            number(4),
            number(9),
            number(5)
        ]
    );
    let entry = append(&mut session, "error(empty),42");
    assert_eq!(
        output(&mut session, entry, vec![Value::Null]),
        vec![number(42)]
    );
}
#[test]
fn errors_preserve_payload_and_stop_after_prior_output() {
    for payload in [
        Value::String(String::new().into()),
        Value::Null,
        Value::Bool(false),
    ] {
        let mut session = Session::new();
        session.bind("payload", payload.clone());
        let entry = append(&mut session, "1,error($payload),2");
        let mut host = InputHost::new(std::iter::empty());
        let mut run = session.run(entry, &mut host, InputMode::Null).unwrap();
        assert_eq!(run.next().unwrap().unwrap(), number(1));
        assert!(matches!(run.next(), Some(Err(JqError::Runtime(value))) if value == payload));
        assert!(run.next().is_none());
        assert_eq!(run.outcome(), Some(&RunOutcome::Error));
        drop(run);
        // A runtime failure does not roll back a successful append in a Session.
        assert!(session.dump(entry).is_ok());
    }
}
#[test]
fn empty_halt_and_cancellation_have_distinct_outcomes() {
    let mut session = Session::new();
    let empty = append(&mut session, "empty");
    assert!(output(&mut session, empty, vec![Value::Null]).is_empty());
    let halt = append(&mut session, "halt,42");
    let mut host = InputHost::new(std::iter::empty());
    let mut run = session.run(halt, &mut host, InputMode::Null).unwrap();
    assert!(run.next().is_none());
    assert_eq!(
        run.outcome(),
        Some(&RunOutcome::Halt {
            code: 0,
            value: Value::Null
        })
    );
    drop(run);
    let entry = append(&mut session, "1,2");
    let mut run = session.run(entry, &mut host, InputMode::Null).unwrap();
    assert!(matches!(run.resume(0), VmEvent::Suspended));
    assert_eq!(run.next().unwrap().unwrap(), number(1));
    run.cancel();
    assert!(run.next().is_none());
    assert_eq!(run.outcome(), Some(&RunOutcome::Cancelled));
}
struct TraceHost {
    calls: Rc<Cell<usize>>,
}
impl Host for TraceHost {
    fn next_input(&mut self) -> Option<Result<Value, JqError>> {
        panic!("null mode must not read input")
    }
    fn environment(&mut self) -> Result<Value, JqError> {
        self.calls.set(self.calls.get() + 1);
        Ok(number(self.calls.get() as i64))
    }
}
#[test]
fn host_effects_are_lazy_and_early_drop_does_not_evaluate_tail() {
    let mut session = Session::new();
    let entry = append(&mut session, "env,env");
    let calls = Rc::new(Cell::new(0));
    let mut host = TraceHost {
        calls: calls.clone(),
    };
    let mut run = session.run(entry, &mut host, InputMode::Null).unwrap();
    assert_eq!(calls.get(), 0);
    assert!(matches!(run.resume(0), VmEvent::Suspended));
    assert_eq!(calls.get(), 0);
    assert_eq!(run.next().unwrap().unwrap(), number(1));
    drop(run);
    assert_eq!(calls.get(), 1);
    assert!(session.dump(entry).is_ok());
}
#[test]
fn outer_loop_and_input_builtins_share_one_cursor() {
    let mut session = Session::new();
    let entry = append(&mut session, ".,input");
    assert_eq!(
        output(&mut session, entry, (1..=4).map(number).collect()),
        (1..=4).map(number).collect::<Vec<_>>()
    );
    let entry = append(&mut session, "inputs,.");
    assert_eq!(
        output(&mut session, entry, (1..=4).map(number).collect()),
        vec![number(2), number(3), number(4), number(1)]
    );
}
#[test]
fn suspended_execution_matches_iterator_and_strings_count_code_points() {
    let mut session = Session::new();
    let entry = append(&mut session, r#""aλ🙂"|length,type"#);
    let mut host = InputHost::new(std::iter::empty());
    let mut run = session.run(entry, &mut host, InputMode::Null).unwrap();
    let mut actual = vec![];
    loop {
        match run.resume(1) {
            VmEvent::Output(value) => actual.push(value),
            VmEvent::Suspended => {}
            VmEvent::Done => break,
            _ => panic!("unexpected event"),
        }
    }
    assert_eq!(
        actual,
        vec![number(3), Value::String("string".to_string().into())]
    );
}
#[test]
fn suspended_ranges_preserve_nested_recovery_and_termination() {
    let mut session = Session::new();
    let entry = append(
        &mut session,
        "range(0;3) | try (range(0;3) | if . == 1 then error else . end) catch .",
    );
    let expected = output(&mut session, entry, vec![Value::Null]);
    assert_eq!(
        expected,
        vec![
            number(0),
            number(1),
            number(0),
            number(1),
            number(0),
            number(1)
        ]
    );
    let mut host = InputHost::new(std::iter::empty());
    let mut run = session.run(entry, &mut host, InputMode::Null).unwrap();
    let mut actual = vec![];
    loop {
        match run.resume(1) {
            VmEvent::Output(value) => actual.push(value),
            VmEvent::Suspended => {}
            VmEvent::Done => break,
            _ => panic!("unexpected event"),
        }
    }
    assert_eq!(actual, expected);
    assert_eq!(run.outcome(), Some(&RunOutcome::Complete));
}

#[test]
fn registry_signatures_docs_and_calc_recipes_are_consistent() {
    let mut signatures = std::collections::HashSet::new();
    for spec in builtins::registry() {
        assert!(signatures.insert((spec.name, spec.params.len())));
        assert!(!spec.docs.is_empty());
        assert_eq!(spec.params.len(), spec.instr.arity());
        if let Some(recipe) = spec.calc {
            assert_eq!(recipe.instr, spec.instr);
            assert_eq!(spec.facts.cardinality, Cardinality::One);
            assert!(!spec.facts.effects.host_io);
            assert!(!spec.facts.effects.nondeterministic);
            assert!(spec.instr.is_scalar());
        }
    }
}
