use oneq::{
    data::{self, Value},
    jq::{
        CompileOptions, Session,
        vm::{InputMode, host::InputHost},
    },
};
fn run(source: &str, input: &str) -> Result<Vec<Value>, String> {
    let mut session = Session::new();
    let entry = session
        .append(source, CompileOptions::default())
        .map_err(|e| e.to_string())?;
    let mut host = InputHost::new(vec![Ok(data::parse_json_str(input).unwrap())].into_iter());
    session
        .run(entry, &mut host, InputMode::Host)
        .unwrap()
        .map(|v| v.map_err(|e| e.to_string()))
        .collect()
}

#[test]
fn direct_builtin_dispatch_preserves_argument_order_and_owned_values() {
    for (source, input, expected) in [
        ("[(10,20) - (1,2)]", "null", "[9,19,8,18]"),
        (
            "[fma((1,2); (10,20); (100,200))]",
            "null",
            "[110,210,120,220,120,220,140,240]",
        ),
        ("[range((0,1); (2,3))]", "null", "[0,1,0,1,2,1,1,2]"),
        ("[range(0;3) | range(0;.)]", "null", "[0,0,1]"),
        (
            "[range(2;2), range(3;1), range(0.5;3)]",
            "null",
            "[0.5,1.5,2.5]",
        ),
        (
            "[try (range(0;4) | if . == 2 then error(\"stop\") else . end) catch .]",
            "null",
            "[0,1,\"stop\"]",
        ),
        (
            "[range(0;3) | try (., error(.)) catch .]",
            "null",
            "[0,0,1,1,2,2]",
        ),
        (
            "[first(range(0;1000000000)), range(0;2)]",
            "null",
            "[0,0,1]",
        ),
        (". as $x | [(. + [3]), $x]", "[1,2]", "[[1,2,3],[1,2]]"),
        ("[path(getpath([\"a\"]))]", "{\"a\":[1]}", "[[\"a\"]]"),
    ] {
        assert_eq!(
            run(source, input).unwrap(),
            vec![data::parse_json_str(expected).unwrap()],
            "{source}"
        );
    }
}

#[test]
fn lazy_iteration_restores_paths_handlers_and_nested_generators() {
    for (source, input, expected) in [
        (
            "[.[] | .[] | (., .+10)]",
            "[[1,2],[],[3]]",
            "[1,11,2,12,3,13]",
        ),
        (
            "[path(.[] | .[])]",
            "{\"a\":[1,2],\"b\":[3]}",
            "[[\"a\",0],[\"a\",1],[\"b\",0]]",
        ),
        (
            "[try (.[] | if . == 2 then error(\"stop\") else . end) catch .]",
            "[1,2,3]",
            "[1,\"stop\"]",
        ),
        (
            "[.[] | try (., error(.)) catch .]",
            "[1,2,3]",
            "[1,1,2,2,3,3]",
        ),
        (
            "[label $out | .[] | if . == 2 then break $out else . end]",
            "[1,2,3]",
            "[1]",
        ),
        (".[] |= .+1", "[1,2,3]", "[2,3,4]"),
        ("[.[] // 9]", "[null,false,2,3]", "[2,3]"),
        ("[.[] // 9]", "[]", "[9]"),
        ("[.[] // 9]", "{}", "[9]"),
        (
            "[def f: if . == 0 then (1,2) else try (.-1|f) catch . end; 100|f]",
            "null",
            "[1,2]",
        ),
    ] {
        assert_eq!(
            run(source, input).unwrap(),
            vec![data::parse_json_str(expected).unwrap()],
            "{source}"
        );
    }
}
#[test]
fn functions_bindings_paths_and_control_flow() {
    for (source, input, expected) in [
        ("[(1,2)+(10,20)]", "null", "[11,12,21,22]"),
        ("map(.+1)", "[1,2,3]", "[2,3,4]"),
        ("[.[]|select(.>1)]", "[0,1,2,3]", "[2,3]"),
        ("[def f($x; g): $x+g; f((1,2);3)]", "null", "[4,5]"),
        (
            "[def f: if .>0 then ., (.-1|f) else . end; f]",
            "3",
            "[3,2,1,0]",
        ),
        ("[(1,2) as $x | def f: $x; f,f]", "null", "[1,1,2,2]"),
        ("[def f: 1; def g: f; def f: 2; g,f]", "null", "[1,2]"),
        (". as [$x, {a:$y}] | [$x,$y]", "[3,{\"a\":4}]", "[3,4]"),
        (".a[.b]", "{\"a\":[10,20],\"b\":1}", "20"),
        (". as $x | $x.a[.b]", "{\"a\":[10,20],\"b\":1}", "20"),
        ("[.[-1], .[1:3], .[:2]]", "[0,1,2,3]", "[3,[1,2],[0,1]]"),
        ("[try (1,error(\"bad\"),2) catch .]", "null", "[1,\"bad\"]"),
        ("[(false,null,2,3) // 9]", "null", "[2,3]"),
        ("[(false,null) // (8,9)]", "null", "[8,9]"),
        ("[.a? // 9]", "1", "[9]"),
        ("[recurse(.[]?)]", "[1,[2]]", "[[1,[2]],1,[2],2]"),
        ("[if (true,false) then . else 9 end]", "7", "[7,9]"),
        ("[try [1,error(\"bad\")] catch ., 4]", "null", "[\"bad\",4]"),
    ] {
        assert_eq!(
            run(source, input).unwrap_or_else(|e| panic!("{source}: {e}")),
            vec![data::parse_json_str(expected).unwrap()],
            "{source}"
        );
    }
    assert!(run("[try (1,2) catch 9 | error(\"outside\")]", "null").is_err());
    assert!(run("[(1 as $x | $x),$x]", "null").is_err());
}
#[test]
fn repl_exports_capture_values_and_survive_redefinition() {
    let mut session = Session::new();
    let opts = CompileOptions {
        repl: true,
        ..CompileOptions::default()
    };
    for (source, expected) in [
        ("(1,2) as $x | def f: $x; empty", "[]"),
        ("[$x, f]", "[[2,2]]"),
        ("9 as $x | [$x, f]", "[[9,2]]"),
        ("def g(p): p,p; [g(f)]", "[[2,2]]"),
        ("[g($x)]", "[[9,9]]"),
    ] {
        let entry = session.append(source, opts.clone()).unwrap();
        let mut host = InputHost::new(std::iter::empty());
        let values = session
            .run(entry, &mut host, InputMode::Null)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            Value::Array(std::rc::Rc::new(values)),
            data::parse_json_str(expected).unwrap(),
            "{source}"
        );
    }
}

#[test]
fn repl_closures_keep_paths_for_future_entries() {
    let mut session = Session::new();
    let opts = CompileOptions {
        repl: true,
        inline: false,
        ..CompileOptions::default()
    };
    let definition = session
        .append(".a as $x | def f: $x; empty", opts.clone())
        .unwrap();
    let mut host = InputHost::new([Ok(data::parse_json_str("{\"a\":7}").unwrap())].into_iter());
    let values = session
        .run(definition, &mut host, InputMode::Host)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(values.is_empty());

    let entry = session.append("path(f)", opts).unwrap();
    let mut host = InputHost::new(std::iter::empty());
    let values = session
        .run(entry, &mut host, InputMode::Null)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(values, vec![data::parse_json_str("[\"a\"]").unwrap()]);
}

#[test]
fn shared_frames_preserve_recursive_and_backtracked_captures_without_inlining() {
    for (source, expected) in [
        (
            "def f: . as $x | if . == 0 then $x else (.-1|f), $x end; [3|f]",
            "[0,1,2,3]",
        ),
        (
            "[(1,2) as $x | def f: $x; (f, (10,20) as $y | f+$y), f]",
            "[1,11,21,1,2,12,22,2]",
        ),
        (
            "def f(g): (1,2) as $x | def h: g+$x; h,h; [10 as $y | f($y)]",
            "[11,11,12,12]",
        ),
        (
            "def outer($x): def middle: def inner: $x; inner; middle; [outer(3),outer(9)]",
            "[3,9]",
        ),
        (
            "def f: . as $x | if .>0 then def g: .-1|f; g, $x else . end; [2|f]",
            "[0,1,2]",
        ),
        (
            "[(1,2) as $x | def f: $x; try (f,error(\"e\")) catch f]",
            "[1,1,2,2]",
        ),
        ("[path(.a as $x | $x[0])]", "[[\"a\",0]]"),
        (
            "[path(.a as $x | ($x[0], $x[], $x))]",
            "[[\"a\",0],[\"a\",0],[\"a\"]]",
        ),
        (
            "[path(.a as $x | def f: $x; (f[0], f))]",
            "[[\"a\",0],[\"a\"]]",
        ),
    ] {
        let mut session = Session::new();
        let entry = session
            .append(
                source,
                CompileOptions {
                    inline: false,
                    ..CompileOptions::default()
                },
            )
            .unwrap();
        let mut host =
            InputHost::new([Ok(data::parse_json_str("{\"a\":[7]}").unwrap())].into_iter());
        let values = session
            .run(entry, &mut host, InputMode::Host)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            values,
            vec![data::parse_json_str(expected).unwrap()],
            "{source}"
        );
    }
}

#[test]
fn folds_preserve_streaming_update_and_extraction_order() {
    for (source, expected) in [
        ("reduce (1,2,3) as $x (0; .+$x)", "6"),
        ("reduce empty as $x (7; .+$x)", "7"),
        ("[reduce (1,2) as $x (0; (.+$x, .+10))]", "[20]"),
        ("[reduce (1,2) as $x (0; empty)]", "[null]"),
        ("[foreach (1,2) as $x (0; (.+$x, .+10))]", "[1,10,12,20]"),
        (
            "[foreach (1,2,3) as $x (0; .+$x; [$x,.])]",
            "[[1,1],[2,3],[3,6]]",
        ),
        ("[reduce (1,2) as $x ((0,10); .+$x)]", "[3,13]"),
        (
            "reduce (1,2) as $x (0; foreach (3,4) as $y (.; .+$y))",
            "14",
        ),
    ] {
        assert_eq!(
            run(source, "null").unwrap(),
            vec![data::parse_json_str(expected).unwrap()],
            "{source}"
        );
    }
}

#[test]
fn standard_collections_regex_formats_and_updates() {
    for (source, input, expected) in [
        ("[range(0;5;2)]", "null", "[0,2,4]"),
        ("[limit(3;range(1000000000))]", "null", "[0,1,2]"),
        ("[limit(4;repeat(.+1))]", "3", "[4,4,4,4]"),
        (
            "[label $out | .[] | if .>2 then break $out else . end]",
            "[1,2,3,4]",
            "[1,2]",
        ),
        (". as {x:$a} ?// [$a] | $a", "[8]", "8"),
        (
            ". as {a:$x} ?// {b:$y} | if $x != null then error(\"retry\") else $y end",
            "{\"a\":1,\"b\":2}",
            "2",
        ),
        (
            "[path(.a),path(.b[])]",
            "{\"a\":1,\"b\":[2,3]}",
            "[[\"a\"],[\"b\",0],[\"b\",1]]",
        ),
        (".a |= (2,3)", "{\"a\":1}", "{\"a\":2}"),
        ("(.a,.b) += 2", "{\"a\":1,\"b\":4}", "{\"a\":3,\"b\":6}"),
        ("del(.[0,2])", "[0,1,2,3]", "[1,3]"),
        (".[] |= select(.>1)", "[0,1,2,3]", "[2,3]"),
        ("setpath([\"a\",2];9)", "null", "{\"a\":[null,null,9]}"),
        (
            "sort_by(.n) | map(.n)",
            "[{\"n\":3},{\"n\":1},{\"n\":2}]",
            "[1,2,3]",
        ),
        ("group_by(.)", "[3,1,2,1]", "[[1,1],[2],[3]]"),
        (
            "with_entries(.value += 1)",
            "{\"a\":1,\"b\":2}",
            "{\"a\":2,\"b\":3}",
        ),
        (
            "[flatten(1),transpose]",
            "[[1,2],[3]]",
            "[[1,2,3],[[1,3],[2,null]]]",
        ),
        (
            "[\"λ🙂\"|explode,utf8bytelength]",
            "null",
            "[[955,128578],6]",
        ),
        ("@uri \"prefix/\\(.)\"", "\"a b\"", "\"prefix/a%20b\""),
        ("@base64 | @base64d", "\"λ🙂\"", "\"λ🙂\""),
        ("@csv", "[\"a\\\"b\",1,null]", "\"\\\"a\\\"\\\"b\\\",1,\""),
        ("[match(\"(?<x>λ)\";\"g\")|.offset]", "\"aλ🙂λ\"", "[1,3]"),
        (
            "capture(\"(?<x>[a-z]+)-(?<n>[0-9]+)\")",
            "\"ab-12\"",
            "{\"x\":\"ab\",\"n\":\"12\"}",
        ),
        (
            "gsub(\"(?<x>[0-9])\"; \"[\\(.x)]\")",
            "\"a1b2\"",
            "\"a[1]b[2]\"",
        ),
        (
            "[scan(\"([a-z])([0-9])\")]",
            "\"a1b2\"",
            "[[\"a\",\"1\"],[\"b\",\"2\"]]",
        ),
        ("[sqrt,pow(2;3),sin]", "4", "[2,8,-0.7568024953079282]"),
    ] {
        assert_eq!(
            run(source, input).unwrap_or_else(|e| panic!("{source}: {e}")),
            vec![data::parse_json_str(expected).unwrap()],
            "{source}"
        );
    }
    assert_eq!(
        run("[\"a\\((1,2))b\\((3,4))\"]", "null").unwrap(),
        vec![data::parse_json_str("[\"a1b3\",\"a2b3\",\"a1b4\",\"a2b4\"]").unwrap()]
    );
    assert!(run("test(\"a(?=b)\")", "\"ab\"").is_err());
}
