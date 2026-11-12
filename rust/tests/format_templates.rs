use activsync::hevy_settings::{validate_description_template, validate_title_template};
use serde_json::Value;

/// Python's own verdicts on each template, produced by running
/// `validate_title_template` and `validate_template` over the inputs.
const TABLE: &str = include_str!("fixtures/format_templates_python.json");

#[test]
fn templates_are_judged_as_python_judges_them() {
    let rows: Vec<Value> = serde_json::from_str(TABLE).unwrap();
    assert!(rows.len() > 50);
    for row in rows {
        let input = row["input"].as_str().unwrap();
        for (kind, actual) in [
            ("title", validate_title_template(input)),
            ("description", validate_description_template(input)),
        ] {
            let expected = row[kind]
                .as_str()
                .map_or(Ok(()), |message| Err(message.to_string()));
            assert_eq!(actual, expected, "{kind} template {input:?}");
        }
    }
}

#[test]
fn an_empty_format_spec_is_accepted_and_a_stray_brace_after_an_unknown_name_is_not_reached() {
    assert_eq!(validate_title_template("{title:}"), Ok(()));
    assert_eq!(
        validate_title_template("{name} }"),
        Err("Unknown title placeholder: {name}.".into())
    );
}
