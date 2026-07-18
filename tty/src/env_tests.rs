use super::*;

#[test]
fn parses_and_sorts_name_value_pairs() {
    let vars = parse(b"PATH=/usr/bin\nHOME=/Users/me\n");
    assert_eq!(
        vars,
        vec![
            EnvVar {
                name: "HOME".into(),
                value: "/Users/me".into()
            },
            EnvVar {
                name: "PATH".into(),
                value: "/usr/bin".into()
            },
        ]
    );
}

#[test]
fn keeps_equals_signs_inside_the_value() {
    // Only the first `=` splits name from value.
    let vars = parse(b"DB_URL=postgres://u:p@h/db?x=1\n");
    assert_eq!(vars.len(), 1);
    assert_eq!(vars[0].name, "DB_URL");
    assert_eq!(vars[0].value, "postgres://u:p@h/db?x=1");
}

#[test]
fn skips_lines_without_an_equals() {
    // A continuation line from a value with an embedded newline has no `=` — skip it
    // rather than invent a variable.
    let vars = parse(b"A=1\nnot a pair\nB=2\n");
    assert_eq!(
        vars.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
        ["A", "B"]
    );
}

#[test]
fn read_missing_file_is_empty_not_an_error() {
    let vars = read(std::path::Path::new("/definitely/not/here/env"));
    assert!(vars.is_empty());
}

#[test]
fn export_command_single_quote_escapes_the_value() {
    assert_eq!(
        export_command("FOO", "bar baz").unwrap(),
        b"export FOO='bar baz'\n"
    );
    // A value containing a single quote is escaped so it can't break out.
    assert_eq!(
        export_command("MSG", "it's \"here\"").unwrap(),
        b"export MSG='it'\\''s \"here\"'\n"
    );
}

#[test]
fn unset_command_is_simple() {
    assert_eq!(unset_command("FOO").unwrap(), b"unset FOO\n");
}

#[test]
fn invalid_names_are_rejected() {
    // A name that could smuggle shell syntax must produce nothing.
    assert!(export_command("FOO; rm -rf /", "x").is_none());
    assert!(export_command("", "x").is_none());
    assert!(export_command("1FOO", "x").is_none());
    assert!(export_command("FO-O", "x").is_none());
    assert!(unset_command("$(evil)").is_none());
    // Legit names pass.
    assert!(export_command("_FOO2", "x").is_some());
}
