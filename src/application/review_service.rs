//! Reviewing a spec before it is provided (#89, the MCP tool `review_spec`):
//! the provide's own validation and version rules, run as a dry run against
//! the stored line, plus the few findings that matter in Sanshain. Nothing is
//! stored, audited or sent. No general style linting: every finding names a
//! consequence inside Sanshain.

use super::provide_service::{ProvideVerdict, review_provide};
use super::spec_service::{
    content_hash, extract_spec_version, normalize_spec_text, parse_spec_endpoints,
};
use super::version_rules::ga_baseline;
use crate::domain::models::{
    ApiType, AppError, EndpointRecord, HarvestedSubscriptionResult, ProvideChanges, SemVer,
    SpecVersionMeta, Stability,
};
use crate::domain::ports::SpecRepository;
use crate::openapi::EndpointSpec;
use serde::Serialize;
use std::collections::HashSet;

/// What a review is asked about.
pub struct ReviewRequest<'a> {
    pub producer: &'a str,
    pub api_type: ApiType,
    pub content: &'a str,
    /// The stability the provide would declare.
    pub stability: Stability,
    /// The user snapshot ownership is checked for; `None` has no owner.
    pub reviewer: Option<&'a str>,
}

/// The provide-side validation: what a provide would read out of the text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Validation {
    pub valid: bool,
    /// Why a provide would answer `400`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub version: Option<SemVer>,
    /// The hash of the normalized text, the one the no-op compares.
    pub content_hash: String,
    pub endpoints: Vec<ReviewedEndpoint>,
}

/// One endpoint as the splitter stores it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReviewedEndpoint {
    pub method: String,
    pub path: String,
    pub deprecated: bool,
}

/// The version line the rules were asked against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LineFacts {
    /// Stored versions of this Producer and API type; 0 for a new line.
    pub versions: usize,
    /// The entry already under this number, if any.
    pub existing: Option<ExistingVersion>,
    /// The highest GA below this number: what removals and the breaking
    /// check compare against.
    pub previous_ga: Option<SemVer>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExistingVersion {
    pub stability: Stability,
    pub provided_by: String,
}

/// What the version rules answer, as a dry run would.
#[derive(Clone, Debug, Serialize)]
pub struct RuleVerdict {
    pub accepted: bool,
    /// The refusing rule: `immutable_ga`, `snapshot_on_ga`, `snapshot_owned`,
    /// `breaking_without_major` or `message_contract_conflict`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_version: Option<SemVer>,
    /// Accepted with the bytes the version already holds: a no-op.
    pub unchanged: bool,
    /// Accepted as GA over a snapshot of the same number: released in place.
    pub promoted: bool,
    /// The endpoint changes against this version's stored entry (all
    /// inserts for a new version).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changes: Option<ProvideChanges>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub harvested_subscriptions: Vec<HarvestedSubscriptionResult>,
    /// The role the build's token needs for this provide.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_role: Option<&'static str>,
    pub line: LineFacts,
}

/// One Sanshain-specific finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Where in the document, for findings that are not about an endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

/// The finding codes, in the order a review answers them.
pub const FINDING_CODES: [&str; 5] = [
    "missing_version_marker",
    "absolute_server_url",
    "unnamed_message",
    "removed_without_deprecation",
    "missing_summary",
];

#[derive(Clone, Debug, Serialize)]
pub struct SpecReview {
    pub producer: String,
    pub api_type: ApiType,
    pub stability: Stability,
    pub validation: Validation,
    /// `None` when the document is invalid: no rule can be asked.
    pub version_rules: Option<RuleVerdict>,
    pub findings: Vec<Finding>,
}

fn finding(code: &'static str, message: String) -> Finding {
    Finding {
        code,
        message,
        method: None,
        path: None,
        location: None,
    }
}

fn at_endpoint(mut f: Finding, method: &str, path: &str) -> Finding {
    f.method = Some(method.to_string());
    f.path = Some(path.to_string());
    f
}

/// The findings the text alone answers, whether or not it is valid.
fn document_findings(api_type: ApiType, content: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    if api_type == ApiType::Proto && !crate::proto::has_version_marker(content) {
        out.push(finding(
            "missing_version_marker",
            "no '// sanshain-version: MAJOR.MINOR.PATCH' comment: proto has no version slot, so \
             every provide of this file is refused"
                .to_string(),
        ));
    }
    let servers = match api_type {
        ApiType::OpenApi => crate::openapi::absolute_server_urls(content),
        ApiType::AsyncApi => crate::asyncapi::absolute_server_urls(content),
        ApiType::Proto => Vec::new(),
    };
    for (location, url) in servers {
        out.push(Finding {
            location: Some(location),
            ..finding(
                "absolute_server_url",
                format!(
                    "'{url}' is copied into every Consumer's snippet and points it at this one \
                     deployment; a relative URL or server variables are portable"
                ),
            )
        });
    }
    if api_type == ApiType::AsyncApi {
        for m in crate::asyncapi::unnamed_messages(content).unwrap_or_default() {
            let why = if m.operation == "PUB" {
                "no message contract is shared for it"
            } else {
                "no subscription is harvested for it"
            };
            out.push(at_endpoint(
                finding(
                    "unnamed_message",
                    format!("a message has neither name nor title: {why}"),
                ),
                m.operation,
                &m.channel,
            ));
        }
    }
    out
}

/// Endpoints of the previous GA that are gone and were not marked
/// deprecated there: the deprecation marker must sit on the removed element.
fn removals(previous: &[EndpointRecord], current: &[EndpointSpec], ga: SemVer) -> Vec<Finding> {
    let kept: HashSet<(&str, &str)> = current
        .iter()
        .map(|e| (e.normalized_path.as_str(), e.method.as_str()))
        .collect();
    previous
        .iter()
        .filter(|e| {
            !e.deprecated && !kept.contains(&(e.normalized_path.as_str(), e.method.as_str()))
        })
        .map(|e| {
            at_endpoint(
                finding(
                    "removed_without_deprecation",
                    format!(
                        "removed without being marked deprecated in GA {ga}: Consumers had no \
                         warning; deprecate it in a GA first"
                    ),
                ),
                &e.method,
                &e.path,
            )
        })
        .collect()
}

fn missing_summaries(api_type: ApiType, endpoints: &[EndpointSpec]) -> Vec<Finding> {
    endpoints
        .iter()
        .filter(|e| crate::descriptions::endpoint_summary(api_type, &e.yaml_content).is_none())
        .map(|e| {
            at_endpoint(
                finding(
                    "missing_summary",
                    "no summary or description: the UI's endpoint lists and the MCP answers \
                     show nothing for it"
                        .to_string(),
                ),
                &e.method,
                &e.path,
            )
        })
        .collect()
}

/// Review a document against its Producer's stored line. Stores nothing.
pub async fn review_spec(
    repo: &impl SpecRepository,
    request: ReviewRequest<'_>,
) -> Result<SpecReview, AppError> {
    let api_type = request.api_type;
    let (parsed, hash, mut findings) = {
        let content = normalize_spec_text(request.content).into_owned();
        let producer = request.producer.to_string();
        super::run_cpu_bound(move || {
            let parsed = extract_spec_version(api_type, &content).and_then(|version| {
                parse_spec_endpoints(api_type, &content, &producer).map(|e| (version, e))
            });
            Ok((
                parsed,
                content_hash(&content),
                document_findings(api_type, &content),
            ))
        })
        .await?
    };
    let (version, endpoints) = match parsed {
        Ok(parsed) => parsed,
        Err(AppError::BadRequest(error)) => {
            return Ok(SpecReview {
                producer: request.producer.to_string(),
                api_type,
                stability: request.stability,
                validation: Validation {
                    valid: false,
                    error: Some(error),
                    version: None,
                    content_hash: hash,
                    endpoints: Vec::new(),
                },
                version_rules: None,
                findings,
            });
        }
        Err(e) => return Err(e),
    };

    let line: Vec<SpecVersionMeta> = match repo.find_service(request.producer).await? {
        Some(sid) => repo
            .list_spec_versions(sid)
            .await?
            .into_iter()
            .filter(|v| v.api_type == api_type)
            .collect(),
        None => Vec::new(),
    };
    let existing = line.iter().find(|v| v.version == version);
    let baseline = ga_baseline(&line, version);
    if let Some(ga) = &baseline {
        let previous = repo.get_endpoints_for_version(ga.id).await?;
        findings.extend(removals(&previous, &endpoints, ga.version));
    }
    findings.extend(missing_summaries(api_type, &endpoints));

    let facts = LineFacts {
        versions: line.len(),
        existing: existing.map(|e| ExistingVersion {
            stability: e.stability,
            provided_by: e.provided_by.clone(),
        }),
        previous_ga: baseline.as_ref().map(|b| b.version),
    };
    let requires_role = (request.stability == Stability::Ga).then_some("releaser");
    let verdict = review_provide(
        repo,
        request.producer,
        api_type,
        request.content,
        request.stability,
        request.reviewer,
    )
    .await?;
    let version_rules = match verdict {
        ProvideVerdict::Accepted(response) => RuleVerdict {
            accepted: true,
            rule: None,
            message: None,
            proposed_version: None,
            unchanged: !response.promoted
                && existing.is_some_and(|e| e.content_hash == response.content_hash),
            promoted: response.promoted,
            changes: Some(response.changes),
            harvested_subscriptions: response.harvested_subscriptions,
            requires_role,
            line: facts,
        },
        ProvideVerdict::Rejected {
            rule,
            message,
            proposed,
        } => RuleVerdict {
            accepted: false,
            rule: Some(rule),
            message: Some(message),
            proposed_version: proposed,
            unchanged: false,
            promoted: false,
            changes: None,
            harvested_subscriptions: Vec::new(),
            requires_role,
            line: facts,
        },
    };
    Ok(SpecReview {
        producer: request.producer.to_string(),
        api_type,
        stability: request.stability,
        validation: Validation {
            valid: true,
            error: None,
            version: Some(version),
            content_hash: hash,
            endpoints: endpoints
                .iter()
                .map(|e| ReviewedEndpoint {
                    method: e.method.clone(),
                    path: e.path.clone(),
                    deprecated: e.deprecated,
                })
                .collect(),
        },
        version_rules: Some(version_rules),
        findings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;
    use crate::application::provide_service::{ProvideSpecParams, provide_spec};
    use crate::domain::permissions::Actor;

    const V1: &str = "openapi: 3.0.3\ninfo:\n  title: orders\n  version: 1.0.0\nservers:\n  - url: https://orders.corp.example\n  - url: /api\npaths:\n  /orders:\n    get:\n      responses:\n        '200': { description: ok }\n  /orders/{id}:\n    get:\n      summary: Fetch one order\n      responses:\n        '200': { description: ok }\n    delete:\n      deprecated: true\n      description: Cancels an order. Soon gone.\n      responses:\n        '204': { description: gone }\n";

    async fn seeded() -> MockRepo {
        let repo = MockRepo::new();
        provide_spec(
            &repo,
            ProvideSpecParams {
                producername: "orders",
                api_type: ApiType::OpenApi,
                content: V1,
                stability: Stability::Ga,
                dry_run: false,
                trunk: false,
                tag: None,
                caller: Some(Actor::test_releaser_named("alice")),
                require_prior_content_match: false,
            },
        )
        .await
        .unwrap();
        repo
    }

    fn request<'a>(api_type: ApiType, content: &'a str, stability: Stability) -> ReviewRequest<'a> {
        ReviewRequest {
            producer: "orders",
            api_type,
            content,
            stability,
            reviewer: Some("bob"),
        }
    }

    fn codes(review: &SpecReview) -> Vec<(&str, Option<&str>, Option<&str>)> {
        review
            .findings
            .iter()
            .map(|f| (f.code, f.method.as_deref(), f.path.as_deref()))
            .collect()
    }

    #[tokio::test]
    async fn a_major_without_a_deprecation_first_passes_the_rules_but_is_a_finding() {
        let repo = seeded().await;
        // 2.0.0 drops GET /orders/{id} (never deprecated) and DELETE (deprecated).
        let v2 = V1
            .replace("version: 1.0.0", "version: 2.0.0")
            .replace(
                "  /orders/{id}:\n    get:\n      summary: Fetch one order\n      responses:\n        '200': { description: ok }\n    delete:\n      deprecated: true\n      description: Cancels an order. Soon gone.\n      responses:\n        '204': { description: gone }\n",
                "",
            );
        let review = review_spec(&repo, request(ApiType::OpenApi, &v2, Stability::Ga))
            .await
            .unwrap();
        assert!(review.validation.valid);
        assert_eq!(review.validation.version, Some(SemVer::new(2, 0, 0)));
        assert_eq!(
            review.validation.endpoints,
            vec![ReviewedEndpoint {
                method: "GET".to_string(),
                path: "/orders".to_string(),
                deprecated: false
            }]
        );
        let rules = review.version_rules.as_ref().unwrap();
        assert!(rules.accepted, "{rules:?}");
        assert!(!rules.unchanged);
        assert_eq!(rules.requires_role, Some("releaser"));
        assert_eq!(rules.changes.as_ref().map(|c| c.inserts), Some(1));
        assert_eq!(
            rules.line,
            LineFacts {
                versions: 1,
                existing: None,
                previous_ga: Some(SemVer::new(1, 0, 0)),
            }
        );
        assert_eq!(
            codes(&review),
            [
                ("absolute_server_url", None, None),
                (
                    "removed_without_deprecation",
                    Some("GET"),
                    Some("/orders/{id}")
                ),
                ("missing_summary", Some("GET"), Some("/orders")),
            ]
        );
        assert_eq!(
            review.findings[0].location.as_deref(),
            Some("servers[0].url")
        );
        assert!(
            review.findings[0]
                .message
                .contains("https://orders.corp.example"),
            "{}",
            review.findings[0].message
        );
        assert!(
            review.findings[1].message.contains("GA 1.0.0"),
            "{}",
            review.findings[1].message
        );

        let sid = repo.find_service("orders").await.unwrap().unwrap();
        assert_eq!(repo.list_spec_versions(sid).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_refusal_and_a_no_op_are_verdicts() {
        let repo = seeded().await;
        let changed = V1.replace("Fetch one order", "Fetch an order");
        let review = review_spec(&repo, request(ApiType::OpenApi, &changed, Stability::Ga))
            .await
            .unwrap();
        let rules = review.version_rules.unwrap();
        assert!(!rules.accepted);
        assert_eq!(rules.rule, Some("immutable_ga"));
        assert_eq!(rules.proposed_version, Some(SemVer::new(1, 0, 1)));
        assert_eq!(
            rules.line.existing,
            Some(ExistingVersion {
                stability: Stability::Ga,
                provided_by: "alice".to_string()
            })
        );

        let same = review_spec(
            &repo,
            request(ApiType::OpenApi, &V1.replace('\n', "\r\n"), Stability::Ga),
        )
        .await
        .unwrap();
        let rules = same.version_rules.unwrap();
        assert!(rules.accepted && rules.unchanged, "{rules:?}");
        assert_eq!(rules.line.previous_ga, None);
    }

    #[tokio::test]
    async fn an_invalid_proto_answers_the_error_and_the_missing_marker() {
        let repo = MockRepo::new();
        let proto = "syntax = \"proto3\";\npackage shop;\nservice Orders {\n  rpc Get (Req) returns (Res);\n}\n";
        let review = review_spec(&repo, request(ApiType::Proto, proto, Stability::Snapshot))
            .await
            .unwrap();
        assert!(!review.validation.valid);
        assert!(
            review
                .validation
                .error
                .as_deref()
                .is_some_and(|e| e.contains("no version marker")),
            "{:?}",
            review.validation.error
        );
        assert!(review.version_rules.is_none());
        assert_eq!(codes(&review), [("missing_version_marker", None, None)]);
        assert_eq!(repo.find_service("orders").await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_referenced_named_message_is_no_unnamed_message_finding() {
        let repo = MockRepo::new();
        let doc = "asyncapi: 2.6.0\ninfo: { title: orders, version: 1.0.0 }\nchannels:\n  orders.placed:\n    publish:\n      summary: An order was placed\n      message:\n        $ref: '#/components/messages/OrderPlaced'\ncomponents:\n  messages:\n    OrderPlaced: { name: OrderPlaced, payload: { type: object } }\n";
        let review = review_spec(&repo, request(ApiType::AsyncApi, doc, Stability::Snapshot))
            .await
            .unwrap();
        assert_eq!(
            codes(&review),
            Vec::<(&str, Option<&str>, Option<&str>)>::new()
        );
    }

    #[tokio::test]
    async fn asyncapi_unnamed_messages_and_a_proto_rpc_without_comment_are_findings() {
        let repo = MockRepo::new();
        let doc = "asyncapi: 2.6.0\ninfo: { title: orders, version: 1.0.0 }\nchannels:\n  orders.placed:\n    publish:\n      summary: An order was placed\n      message:\n        payload: { type: object }\n";
        let review = review_spec(&repo, request(ApiType::AsyncApi, doc, Stability::Snapshot))
            .await
            .unwrap();
        assert_eq!(
            codes(&review),
            [("unnamed_message", Some("PUB"), Some("orders.placed"))]
        );
        assert!(
            review.findings[0].message.contains("no message contract"),
            "{}",
            review.findings[0].message
        );
        let rules = review.version_rules.unwrap();
        assert!(rules.accepted);
        assert_eq!(rules.requires_role, None);
        assert_eq!(rules.line.versions, 0);

        let proto = "// sanshain-version: 1.0.0\nsyntax = \"proto3\";\npackage shop;\nservice Orders {\n  // Fetch one order.\n  rpc Get (Req) returns (Res);\n  rpc List (Req) returns (Res);\n}\n";
        let review = review_spec(&repo, request(ApiType::Proto, proto, Stability::Snapshot))
            .await
            .unwrap();
        assert_eq!(
            codes(&review),
            [("missing_summary", Some("List"), Some("Orders"))]
        );
    }
}
