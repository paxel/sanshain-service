//! The one place YAML is read and written: serde-saphyr (pure Rust, no
//! `unsafe`) with the options the specs need. Untyped documents become a
//! `serde_json::Value`, whose maps keep the document's key order
//! (`preserve_order`).

use serde::Serialize;
use serde::de::DeserializeOwned;

/// A YAML document read without a target type.
pub type Value = serde_json::Value;
/// A YAML mapping of a [`Value`].
pub type Mapping = serde_json::Map<String, serde_json::Value>;

/// Parser options. Only `true` and `false` are booleans (YAML 1.2), so an
/// enum value or example such as `no` or `on` stays the string it was
/// written as. Duplicate keys are an error. The node and event budgets are
/// raised from the library's defaults so a spec at the 4 MiB body limit
/// still parses. Nesting keeps the library's limit of 64 levels: deeper
/// documents would exhaust a worker thread's stack while being read; alias
/// and anchor limits keep their defaults too.
fn options() -> serde_saphyr::Options {
    serde_saphyr::options! {
        strict_booleans: true,
        budget: serde_saphyr::budget! {
            max_nodes: 4_000_000,
            max_events: 8_000_000,
        },
    }
}

/// Parse one YAML document.
pub fn from_str<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    serde_saphyr::from_str_with_options(text, options()).map_err(|e| e.to_string())
}

/// Parse every document of a multi-document YAML stream (`---` separated).
pub fn from_str_multiple<T: DeserializeOwned>(text: &str) -> Result<Vec<T>, String> {
    serde_saphyr::from_str_multiple_with_options(text, options()).map_err(|e| e.to_string())
}

/// Write `value` as one YAML document.
pub fn to_string<T: Serialize>(value: &T) -> Result<String, String> {
    serde_saphyr::to_string(value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_true_and_false_are_booleans() {
        let value: Value = from_str("a: no\nb: on\nc: true\nd: 'yes'\n").unwrap();
        assert_eq!(value["a"], "no");
        assert_eq!(value["b"], "on");
        assert_eq!(value["c"], true);
        assert_eq!(value["d"], "yes");
    }

    #[test]
    fn keys_keep_the_document_order() {
        let value: Value = from_str("zeta: 1\nalpha: 2\nmid: 3\n").unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["zeta", "alpha", "mid"]);
    }

    #[test]
    fn integer_keys_read_as_their_spelling() {
        let value: Value = from_str("responses:\n  200:\n    description: ok\n").unwrap();
        assert_eq!(value["responses"]["200"]["description"], "ok");
    }

    #[test]
    fn duplicate_keys_are_refused() {
        assert!(from_str::<Value>("a: 1\na: 2\n").is_err());
    }

    #[test]
    fn aliases_resolve_to_their_anchor() {
        let value: Value = from_str("base: &b {x: 1}\ncopy: *b\n").unwrap();
        assert_eq!(value["copy"]["x"], 1);
    }

    #[test]
    fn every_document_of_a_stream_is_read() {
        let docs: Vec<Value> = from_str_multiple("a: 1\n---\nb: 2\n").unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[1]["b"], 2);
    }

    #[test]
    fn nesting_is_limited_to_64_levels() {
        let nested = |depth: usize| format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        assert!(from_str::<Value>(&nested(60)).is_ok());
        assert!(from_str::<Value>(&nested(200)).is_err());
    }

    #[test]
    fn written_yaml_reads_back_to_the_same_value() {
        let value: Value = from_str("a: no\nb: [1, two]\nc: {d: 'x: y'}\n").unwrap();
        let text = to_string(&value).unwrap();
        assert_eq!(from_str::<Value>(&text).unwrap(), value);
    }
}
