use sanshain_service::openapi::{split_openapi, EndpointSpec};

fn one_path_yaml(method: &str) -> String {
    format!(
        "openapi: 3.0.0\ninfo: {{ title: T, version: 1.0.0 }}\npaths:\n  /x:\n    {}:\n      responses:\n        '200': {{ description: OK }}\n",
        method
    )
}

// 1. splits single GET endpoint
#[test]
fn split_single_get() {
    let y = one_path_yaml("get");
    let eps = split_openapi(&y).unwrap();
    assert_eq!(eps.len(), 1);
    let e: &EndpointSpec = &eps[0];
    assert_eq!(e.path, "/x");
    assert_eq!(e.method, "GET");
    assert!(e.yaml_content.contains("paths:"));
}

// 2. splits multiple methods on same path
#[test]
fn split_multiple_methods() {
    let y = r#"openapi: 3.0.0
info: { title: T, version: 1.0.0 }
paths:
  /x:
    get:
      responses:
        '200': { description: OK }
    post:
      responses:
        '201': { description: Created }
"#;
    let mut eps = split_openapi(y).unwrap();
    eps.sort_by(|a,b| a.method.cmp(&b.method));
    let methods: Vec<_> = eps.iter().map(|e| e.method.as_str()).collect();
    assert_eq!(methods, vec!["GET", "POST"]);
}

// 3. normalized_path replaces vars and trims slash
#[test]
fn split_sets_normalized_path() {
    let y = r#"openapi: 3.0.0
info: { title: T, version: 1.0.0 }
paths:
  /users/{id}/:
    get:
      responses:
        '200': { description: OK }
"#;
    let eps = split_openapi(y).unwrap();
    // normalize_path trims trailing slash and replaces variables with {}
    assert_eq!(eps[0].normalized_path, "/users/{}");
    assert!(eps[0].normalized_path.contains("{}"));
}

// 4. components are included in yaml content
#[test]
fn split_includes_components() {
    let y = r#"openapi: 3.0.0
info: { title: T, version: 1.0.0 }
paths:
  /x:
    get:
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Thing'
components:
  schemas:
    Thing:
      type: object
      properties: { id: { type: string } }
"#;
    let eps = split_openapi(y).unwrap();
    assert!(eps[0].yaml_content.contains("components:"));
    assert!(eps[0].yaml_content.contains("schemas:"));
}

// 5. invalid yaml returns Err
#[test]
fn split_invalid_yaml_errors() {
    let y = "not: [valid"; // broken
    let err = split_openapi(y).err().unwrap();
    assert!(err.to_lowercase().contains("parse"));
}

// 6. ignores reference-only path items (if any)
#[test]
fn split_ignores_referenced_path_items() {
    // simulate a $ref path; split_openapi will skip Reference variants
    let y = r#"openapi: 3.0.0
info: { title: T, version: 1.0.0 }
paths:
  /x:
    $ref: '#/components/pathItems/X'
components:
  pathItems:
    X:
      get:
        responses:
          '200': { description: OK }
"#;
    let eps = split_openapi(y).unwrap();
    assert!(eps.is_empty());
}
