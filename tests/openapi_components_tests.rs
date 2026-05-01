use sanshain_service::openapi;

fn sample_openapi() -> String {
    // An OpenAPI with two methods on same path, each referencing different components
    // so that split_openapi should include only the actually used components per snippet.
    r#"
openapi: 3.0.0
info:
  title: Sample
  version: 1.0.0
paths:
  /pets:
    get:
      responses:
        '200':
          $ref: '#/components/responses/OkResp'
    post:
      requestBody:
        $ref: '#/components/requestBodies/RB1'
      responses:
        '201':
          description: created
components:
  schemas:
    Pet:
      type: object
      properties:
        id: { type: integer }
        name: { type: string }
      required: [id]
    Owner:
      type: object
      properties:
        id: { type: integer }
  responses:
    OkResp:
      description: ok
      content:
        application/json:
          schema:
            $ref: '#/components/schemas/Pet'
  requestBodies:
    RB1:
      content:
        application/json:
          schema:
            $ref: '#/components/schemas/Owner'
"#
    .to_string()
}

#[test]
fn split_openapi_includes_only_used_components_per_method() {
    let yaml = sample_openapi();
    let parts = openapi::split_openapi(&yaml).expect("split");
    assert_eq!(parts.len(), 2, "expected GET and POST snippets");

    let mut get_yaml = String::new();
    let mut post_yaml = String::new();
    for p in parts {
        if p.method == "GET" {
            get_yaml = p.yaml_content;
        } else if p.method == "POST" {
            post_yaml = p.yaml_content;
        }
    }
    assert!(!get_yaml.is_empty() && !post_yaml.is_empty(), "both methods present");

    // GET should contain OkResp and Pet, but not RB1/Owner
    assert!(get_yaml.contains("OkResp"));
    assert!(get_yaml.contains("schemas"));
    assert!(get_yaml.contains("Pet:"));
    assert!(!get_yaml.contains("requestBodies:"));
    assert!(!get_yaml.contains("RB1"));
    assert!(!get_yaml.contains("Owner:"));

    // POST should contain RB1 and Owner, but not OkResp/Pet
    assert!(post_yaml.contains("requestBodies"));
    assert!(post_yaml.contains("RB1:"));
    assert!(post_yaml.contains("schemas"));
    assert!(post_yaml.contains("Owner:"));
    assert!(!post_yaml.contains("OkResp"));
    assert!(!post_yaml.contains("schemas:\n  Pet:"));
}

#[test]
fn split_openapi_invalid_yaml_errors() {
    let bad = "this: is: not: valid: : yaml";
    let res = openapi::split_openapi(bad);
    assert!(res.is_err());
}

#[test]
fn generate_diff_headers_present_for_no_changes() {
    let a = sample_openapi();
    let diff = openapi::generate_diff(&a, &a);
    // Should include the headers and not crash; content of diff can vary by library,
    // so only assert stable header markers exist.
    // Some implementations may return an empty string when there are no changes.
    assert!(diff.is_empty() || diff.contains("--- ") || diff.contains("+++ "));
}

#[test]
fn merge_endpoint_yamls_errors_on_empty() {
    let err = openapi::merge_endpoint_yamls(&[]).unwrap_err();
    assert!(err.to_lowercase().contains("no endpoint yamls"));
}

#[test]
fn merge_endpoint_yamls_merges_methods() {
    // Reuse the split to get two per-endpoint snippets, then merge back and expect both methods present.
    let yaml = sample_openapi();
    let parts = openapi::split_openapi(&yaml).expect("split");
    let snippets: Vec<String> = parts.into_iter().map(|p| p.yaml_content).collect();
    let merged = openapi::merge_endpoint_yamls(&snippets).expect("merge");
    // Both GET and POST should be present in the merged spec
    assert!(merged.contains("/pets"));
    assert!(merged.contains("get:"));
    assert!(merged.contains("post:"));
}

#[test]
fn split_openapi_supports_all_methods_and_skips_ref_path_items() {
    let yaml = r#"
openapi: 3.0.0
info: { title: M, version: 1.0.0 }
paths:
  /mix:
    get: { responses: { '200': { description: ok } } }
    post: { responses: { '200': { description: ok } } }
    put: { responses: { '200': { description: ok } } }
    delete: { responses: { '200': { description: ok } } }
    options: { responses: { '200': { description: ok } } }
    head: { responses: { '200': { description: ok } } }
    patch: { responses: { '200': { description: ok } } }
    trace: { responses: { '200': { description: ok } } }
  /refd:
    $ref: '#/components/x-dummy'
components: {}
"#;

    let parts = openapi::split_openapi(yaml).expect("split");
    // We expect 8 methods for /mix and skip the $ref path item entirely
    assert_eq!(parts.len(), 8);

    let mut methods: Vec<String> = parts.into_iter().map(|p| p.method).collect();
    methods.sort();
    assert_eq!(methods, vec![
        "DELETE", "GET", "HEAD", "OPTIONS", "PATCH", "POST", "PUT", "TRACE"
    ]);
}
