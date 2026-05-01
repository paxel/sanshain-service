use sanshain_service::openapi;

#[test]
fn backward_compatibility_breaks_on_removed_status() {
    let old = r#"
openapi: 3.0.0
info: { title: T, version: 1.0.0 }
paths:
  /x:
    get:
      responses:
        '200': { description: OK }
"#;
    let new = r#"
openapi: 3.0.0
info: { title: T, version: 1.0.1 }
paths:
  /x:
    get:
      responses:
        '201': { description: Created }
"#;
    let res = openapi::check_backward_compatibility(old, new);
    assert!(res.is_err());
    let msg = res.err().unwrap();
    assert!(msg.contains("Response status"));
}

#[test]
fn generate_diff_has_headers_and_markers() {
    let old = "a\nline\n";
    let new = "a\nline2\n";
    let diff = openapi::generate_diff(old, new);
    assert!(diff.contains("--- previous"));
    assert!(diff.contains("+++ current"));
    assert!(diff.contains("-line"));
    assert!(diff.contains("+line2"));
}
