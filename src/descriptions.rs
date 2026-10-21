//! One-line descriptions of endpoints, read out of the stored snippets.
//!
//! Producers describe their operations in the document itself (an OpenAPI
//! `summary`, an AsyncAPI message summary, a comment above a proto `rpc`).
//! Nothing stores that separately: every endpoint row carries its own
//! snippet, and the first sentence of what the document says is derived
//! from it when a list or an assistant asks "what is this for".

use crate::domain::models::ApiType;

/// How long a derived summary may be before it is cut.
const MAX_SUMMARY_CHARS: usize = 160;

/// The first sentence of a description: up to the first sentence-ending
/// punctuation followed by whitespace, or the first paragraph, whichever
/// comes first, cut at [`MAX_SUMMARY_CHARS`] with an ellipsis. Whitespace is
/// collapsed; `None` for blank text.
pub fn first_sentence(text: &str) -> Option<String> {
    let paragraph = text
        .trim()
        .split("\n\n")
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if paragraph.is_empty() {
        return None;
    }
    let mut sentence = paragraph.as_str();
    let chars: Vec<(usize, char)> = paragraph.char_indices().collect();
    for (i, (idx, ch)) in chars.iter().enumerate() {
        if matches!(ch, '.' | '!' | '?')
            && chars
                .get(i + 1)
                .is_some_and(|(_, next)| next.is_whitespace())
        {
            sentence = &paragraph[..idx + ch.len_utf8()];
            break;
        }
    }
    let mut out: String = sentence.chars().take(MAX_SUMMARY_CHARS).collect();
    if sentence.chars().count() > MAX_SUMMARY_CHARS {
        out = out.trim_end().to_string();
        out.push('…');
    }
    Some(out)
}

/// The one-line description of an endpoint snippet of the given type, if the
/// Producer wrote one.
pub fn endpoint_summary(api_type: ApiType, snippet: &str) -> Option<String> {
    match api_type {
        ApiType::OpenApi => crate::openapi::operation_summary(snippet),
        ApiType::AsyncApi => crate::asyncapi::channel_summary(snippet),
        ApiType::Proto => crate::proto::rpc_summary(snippet),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sentence_cuts_at_the_sentence_end_or_paragraph() {
        assert_eq!(
            first_sentence("Lists orders. Paged by cursor.").as_deref(),
            Some("Lists orders.")
        );
        assert_eq!(
            first_sentence("Lists orders\n\nMore prose.").as_deref(),
            Some("Lists orders")
        );
        assert_eq!(
            first_sentence("  Lists   orders\n  by id ").as_deref(),
            Some("Lists orders by id")
        );
        assert_eq!(
            first_sentence("v1.2 of the API is here").as_deref(),
            Some("v1.2 of the API is here")
        );
        assert_eq!(first_sentence("   \n "), None);
    }

    #[test]
    fn first_sentence_cuts_long_text_with_an_ellipsis() {
        let long = "x".repeat(200);
        let cut = first_sentence(&long).unwrap();
        assert_eq!(cut.chars().count(), MAX_SUMMARY_CHARS + 1);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn endpoint_summary_dispatches_by_type() {
        let openapi = "openapi: 3.0.3\npaths:\n  /a:\n    get:\n      summary: Reads A\n";
        assert_eq!(
            endpoint_summary(ApiType::OpenApi, openapi).as_deref(),
            Some("Reads A")
        );
        let asyncapi =
            "asyncapi: 2.6.0\nchannels:\n  a.events:\n    publish:\n      summary: Emits A\n";
        assert_eq!(
            endpoint_summary(ApiType::AsyncApi, asyncapi).as_deref(),
            Some("Emits A")
        );
        let proto = "service S {\n  // Reads A.\n  rpc GetA (Req) returns (Res);\n}\n";
        assert_eq!(
            endpoint_summary(ApiType::Proto, proto).as_deref(),
            Some("Reads A.")
        );
        assert_eq!(
            endpoint_summary(ApiType::OpenApi, "openapi: 3.0.3\npaths: {}\n"),
            None
        );
    }
}
