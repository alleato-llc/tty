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
