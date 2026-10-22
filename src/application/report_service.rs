use crate::domain::models::*;
use crate::domain::ports::SpecRepository;
use tracing::instrument;

/// The graph a report describes (ADR-0005): the accumulated dev activity
/// (default), the trunk pin set, or a sanshain-branch — main and branches
/// optionally at a past instant (`main@<rfc3339>`, `<branch>@<rfc3339>`).
pub enum ReportScope {
    Dev,
    Main { at: Option<String> },
    Branch { name: String, at: Option<String> },
}

impl ReportScope {
    /// `scope=dev|main[@rfc3339]|<branch>[@rfc3339]` — the same selector
    /// grammar the graph-diff endpoint speaks (see
    /// [`super::branch_service::split_selector`]).
    pub fn parse(raw: Option<&str>) -> Result<ReportScope, AppError> {
        let Some(raw) = raw else {
            return Ok(ReportScope::Dev);
        };
        match super::branch_service::split_selector(raw)? {
            ("dev", None) => Ok(ReportScope::Dev),
            ("dev", Some(_)) => Err(AppError::BadRequest(
                "the dev scope is accumulated activity and has no timeline — \
                 read main or a sanshain-branch at an instant instead"
                    .to_string(),
            )),
            ("main", at) => Ok(ReportScope::Main { at }),
            (name, at) => Ok(ReportScope::Branch {
                name: name.to_string(),
                at,
            }),
        }
    }
}

/// The periphery edges (#46) of a graph scope: the dev set, or trunk's or a
/// branch's set — current or as it was at the scope's instant, so a past view
/// never shows today's resources.
#[instrument(skip_all)]
pub async fn get_periphery(
    repo: &impl SpecRepository,
    scope: &ReportScope,
) -> Result<Vec<PeripheralDependency>, AppError> {
    let (track, at) = match scope {
        ReportScope::Dev => (PeripheryTrack::Dev, None),
        ReportScope::Main { at } => (PeripheryTrack::Trunk, at.as_deref()),
        ReportScope::Branch { name, at } => {
            let branch = repo
                .find_branch(name)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("no sanshain-branch '{name}'")))?;
            (PeripheryTrack::Branch(branch.id), at.as_deref())
        }
    };
    without_known_components(repo, repo.list_periphery(track, at).await?).await
}

/// ADR-0010: a `component:` ref is drawn as a missing component only while no
/// Producer or Consumer of that name exists; with a pendant the `require`
/// edges are the truth and the ref is dropped. Present-day names, whatever
/// the scope: a system that exists today is not missing in a cut either.
pub async fn without_known_components(
    repo: &impl SpecRepository,
    periphery: Vec<PeripheralDependency>,
) -> Result<Vec<PeripheralDependency>, AppError> {
    if !periphery
        .iter()
        .any(|p| p.resource.starts_with("component:"))
    {
        return Ok(periphery);
    }
    let mut known: std::collections::HashSet<String> = repo
        .list_producers_detailed()
        .await?
        .into_iter()
        .map(|p| p.name.to_lowercase())
        .collect();
    known.extend(
        repo.list_consumers()
            .await?
            .into_iter()
            .map(|c| c.to_lowercase()),
    );
    Ok(periphery
        .into_iter()
        .filter(|p| {
            crate::backstage::component_ref_name(&p.resource)
                .is_none_or(|name| !known.contains(name))
        })
        .collect())
}

/// The harvested subscriptions (ADR-0006) that belong to a scope: dev keeps
/// all of them. Harvested edges are present-tense, so outside dev only the
/// trunk ones survive, and only for the *live* main view — a past
/// `main@<date>` or a branch view has no harvested timeline yet, so it carries
/// none rather than masquerading today's edges as historical.
pub fn scope_harvested_subscriptions(
    scope: &ReportScope,
    mut subscriptions: Vec<HarvestedSubscription>,
) -> Vec<HarvestedSubscription> {
    match scope {
        ReportScope::Dev => {}
        ReportScope::Main { at: None } => subscriptions.retain(|h| h.trunk),
        _ => subscriptions.clear(),
    }
    subscriptions
}

/// One version line found in a scope before its stored entry is looked up.
struct ScopedLine {
    service: String,
    api_type: ApiType,
    version: SemVer,
    /// When the scope's own record says it was last provided (trunk member
    /// versions), or that a provide made it current (branch members).
    recorded_provide: Option<String>,
}

/// The latest provide of a line known at the scope's instant: the later of
/// the stored entry's `updated_at` (its last provide, any stream) and the
/// scope's own record of a provide — at an instant, only those not after it.
/// `None` when no provide at or before the instant is known.
pub fn last_provided_at_instant(
    updated_at: Option<&str>,
    recorded_provide: Option<&str>,
    at: Option<&str>,
) -> Option<String> {
    recorded_provide
        .into_iter()
        .chain(updated_at)
        .filter(|t| at.is_none_or(|at| *t <= at))
        .max()
        .map(str::to_string)
}

/// The producers of a graph scope at its instant, each with its version lines
/// there (one per API type), plus the scope's harvested subscriptions. Dev:
/// every producer with any version, its newest version per line. Main: trunk's
/// member versions (every trunk-flagged Provide records one). Branch: the
/// branch's member versions, and for lines without one the highest version the
/// branch pins — membership is pins and member versions alike (ADR-0005).
#[instrument(skip_all)]
pub async fn get_graph_scope(
    repo: &impl SpecRepository,
    scope: &ReportScope,
) -> Result<GraphScopeData, AppError> {
    use std::collections::{BTreeMap, HashMap};
    let entries: HashMap<(String, ApiType, SemVer), SpecVersionMeta> = repo
        .list_all_spec_versions()
        .await?
        .into_iter()
        .map(|(name, meta, _)| ((name, meta.api_type, meta.version), meta))
        .collect();
    let mut lines: BTreeMap<(String, &'static str), ScopedLine> = BTreeMap::new();
    let mut keep_highest = |line: ScopedLine| {
        let key = (line.service.clone(), line.api_type.as_str());
        match lines.get(&key) {
            Some(existing) if existing.version >= line.version => {}
            _ => {
                lines.insert(key, line);
            }
        }
    };
    let at = match scope {
        ReportScope::Dev => {
            for (name, api_type, version) in entries.keys() {
                keep_highest(ScopedLine {
                    service: name.clone(),
                    api_type: *api_type,
                    version: *version,
                    recorded_provide: None,
                });
            }
            None
        }
        ReportScope::Main { at } => {
            for m in repo
                .list_member_versions(MemberTrack::Trunk, at.as_deref())
                .await?
            {
                keep_highest(ScopedLine {
                    service: m.service,
                    api_type: m.api_type,
                    version: m.version,
                    recorded_provide: Some(match (&m.last_provided_at, at) {
                        (Some(last), Some(at)) if last.as_str() > at.as_str() => m.valid_from,
                        (Some(last), _) => last.clone(),
                        (None, _) => m.valid_from,
                    }),
                });
            }
            at.as_deref()
        }
        ReportScope::Branch { name, at } => {
            let branch = repo
                .find_branch(name)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("no sanshain-branch '{name}'")))?;
            let members = repo
                .list_member_versions(MemberTrack::Branch(branch.id), at.as_deref())
                .await?;
            for pin in repo.list_branch_pins(branch.id, at.as_deref()).await? {
                keep_highest(ScopedLine {
                    service: pin.service,
                    api_type: pin.api_type,
                    version: pin.version,
                    recorded_provide: None,
                });
            }
            // The member version is the line's version in the branch, whatever
            // the pins say — it replaces, never competes.
            for m in members {
                lines.insert(
                    (m.service.clone(), m.api_type.as_str()),
                    ScopedLine {
                        service: m.service,
                        api_type: m.api_type,
                        version: m.version,
                        recorded_provide: Some(m.valid_from),
                    },
                );
            }
            at.as_deref()
        }
    };
    let mut producers: Vec<ScopeProducer> = Vec::new();
    for line in lines.into_values() {
        let entry = entries.get(&(line.service.clone(), line.api_type, line.version));
        let channels = match entry {
            Some(meta) if line.api_type == ApiType::AsyncApi => {
                let mut channels: Vec<ScopeChannel> = repo
                    .get_endpoints_for_version(meta.id)
                    .await?
                    .into_iter()
                    .map(|e| ScopeChannel {
                        channel: e.path,
                        method: e.method,
                    })
                    .collect();
                channels.sort_by(|a, b| (&a.channel, &a.method).cmp(&(&b.channel, &b.method)));
                channels.dedup();
                channels
            }
            _ => Vec::new(),
        };
        let version_line = ScopeVersionLine {
            api_type: line.api_type,
            version: line.version,
            stability: entry.map(|e| e.stability),
            last_provided_at: last_provided_at_instant(
                entry.map(|e| e.updated_at.as_str()),
                line.recorded_provide.as_deref(),
                at,
            ),
            channels,
        };
        match producers.last_mut() {
            Some(p) if p.name == line.service => p.versions.push(version_line),
            _ => producers.push(ScopeProducer {
                name: line.service,
                versions: vec![version_line],
            }),
        }
    }
    let harvested_subscriptions =
        scope_harvested_subscriptions(scope, repo.list_current_harvested_subscriptions().await?);
    Ok(GraphScopeData {
        producers,
        harvested_subscriptions,
        systems: repo.list_system_contexts().await?,
    })
}

/// Project a pin set into the dependency-graph shape reports render. The
/// stability is looked up from the stored version line; a dangling pin
/// (deleted version) reports as GA — reports state pins, not resolutions.
async fn pins_as_dependency_graph(
    repo: &impl SpecRepository,
    pins: Vec<TrunkPinInfo>,
) -> Result<Vec<DependencyInfo>, AppError> {
    // A pin set names the same (service, version) many times; resolve each
    // service and version line once instead of once per pin.
    let mut service_ids: std::collections::HashMap<String, Option<i64>> = Default::default();
    let mut stabilities: std::collections::HashMap<(i64, ApiType, SemVer), Stability> =
        Default::default();
    let mut graph = Vec::with_capacity(pins.len());
    for pin in pins {
        let sid = match service_ids.get(&pin.service) {
            Some(cached) => *cached,
            None => {
                let looked_up = repo.find_service(&pin.service).await?;
                service_ids.insert(pin.service.clone(), looked_up);
                looked_up
            }
        };
        let stability = match sid {
            Some(sid) => match stabilities.get(&(sid, pin.api_type, pin.version)) {
                Some(cached) => *cached,
                None => {
                    let looked_up = repo
                        .find_spec_version(sid, pin.api_type, pin.version)
                        .await?
                        .map(|v| v.stability)
                        .unwrap_or(Stability::Ga);
                    stabilities.insert((sid, pin.api_type, pin.version), looked_up);
                    looked_up
                }
            },
            None => Stability::Ga,
        };
        graph.push(DependencyInfo {
            api_type: pin.api_type,
            client: pin.client,
            service: pin.service,
            version: pin.version,
            stability,
            path: pin.path,
            method: pin.method,
            deprecated: false,
        });
    }
    Ok(graph)
}

/// A report of the selected graph. Dev is byte-identical to the unscoped
/// report; main and branch scopes replace the dependency graph with the
/// scope's pin set (the dev-only overlays are emptied — they describe
/// recorded activity, not a pin set).
pub async fn generate_scoped_report(
    repo: &impl SpecRepository,
    scope: ReportScope,
) -> Result<DependencyReport, AppError> {
    let mut report = generate_report(repo).await?;
    // Outside the dev scope the trunk block is present-tense data that would
    // contradict the scope stamped beside it: `main@<past>` would carry the
    // edges trunk has *today*, which is exactly the masquerade `scope_label`
    // exists to prevent. Emptied like the other dev-only overlays; a caller
    // that wants live trunk asks for the dev scope.
    if !matches!(scope, ReportScope::Dev) {
        report.trunk_graph = Vec::new();
        report.trunk_stale_before = None;
        // Harvested edges are present-tense too. Keep only the trunk ones, and
        // only for the *live* main view — a past `main@<date>` or a branch view
        // has no harvested timeline yet, so it carries none rather than
        // masquerading today's edges as historical.
        report.harvested_subscriptions = scope_harvested_subscriptions(
            &scope,
            std::mem::take(&mut report.harvested_subscriptions),
        );
    }
    if !matches!(scope, ReportScope::Dev) {
        report.peripheral_dependencies = get_periphery(repo, &scope).await?;
    }
    match scope {
        ReportScope::Dev => {}
        ReportScope::Main { at } => {
            let pins = match at.as_deref() {
                Some(at) => repo.list_trunk_pins_at(at).await?,
                None => repo.list_current_trunk_pins().await?,
            };
            report.dependency_graph = pins_as_dependency_graph(repo, pins).await?;
            report.missing_endpoints = Vec::new();
            report.unused_endpoints = Vec::new();
            report.scope_label = Some(match at {
                Some(at) => format!("main@{at}"),
                None => "main".to_string(),
            });
        }
        ReportScope::Branch { name, at } => {
            let branch = repo
                .find_branch(&name)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("no sanshain-branch '{name}'")))?;
            let pins = repo.list_branch_pins(branch.id, at.as_deref()).await?;
            report.dependency_graph = pins_as_dependency_graph(repo, pins).await?;
            report.missing_endpoints = Vec::new();
            report.unused_endpoints = Vec::new();
            report.scope_label = Some(match at {
                Some(at) => format!("{name}@{at}"),
                None => name,
            });
        }
    }
    Ok(report)
}

#[instrument(skip_all)]
pub async fn generate_report(repo: &impl SpecRepository) -> Result<DependencyReport, AppError> {
    let mut report = repo.get_report().await?;
    report.service_tags = repo.get_all_service_tags().await?;
    report.trunk_graph = repo.list_current_trunk_pins().await?;
    // Harvested AsyncAPI subscription edges (#6/ADR-0006). The dev report
    // carries the full set; scoped views filter it below.
    report.harvested_subscriptions = repo.list_current_harvested_subscriptions().await?;
    report.peripheral_dependencies =
        without_known_components(repo, repo.list_periphery(PeripheryTrack::Dev, None).await?)
            .await?;
    // Owner, description, source location and links per system (#48): a
    // present-day fact, so every scope carries the same map.
    report.systems = repo.list_system_contexts().await?;
    // A pin on a deleted version must look broken in the main view too, not
    // only in branch view (ADR-0005: dangling, never silently healthy).
    super::branch_service::mark_dangling(repo, &mut report.trunk_graph).await?;
    // The staleness boundary for the main graph (ADR-0004): trunk entries not
    // refreshed since half the TTL are highlighted as forgotten-in-progress.
    let ttl_days = super::admin_service::get_trunk_max_age_days(repo).await?;
    if ttl_days > 0 {
        let boundary = chrono::Utc::now() - chrono::Duration::hours(ttl_days as i64 * 12);
        report.trunk_stale_before =
            Some(boundary.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
    Ok(report)
}

pub fn render_report_markdown(report: &DependencyReport) -> String {
    let mut md = String::new();
    md.push_str("# Sanshain Dependency Report\n\n");
    if let Some(scope) = &report.scope_label {
        md.push_str(&format!("Scope: {scope}\n\n"));
    }

    md.push_str("| Consumer | Producer | Type | Version | Stability | Path | Method |\n");
    md.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");

    for dep in &report.dependency_graph {
        md.push_str(&format!(
            "| {} | {} | {:?} | {} | {} | `{}` | `{}` |\n",
            dep.client,
            dep.service,
            dep.api_type,
            dep.version,
            dep.stability.as_str(),
            dep.path,
            dep.method
        ));
    }

    md
}

pub fn render_isolation_report(report: &DependencyReport) -> String {
    use std::collections::{BTreeMap, BTreeSet};

    let mut md = String::new();
    md.push_str("# Service Isolation Report\n\n");
    if let Some(scope) = &report.scope_label {
        md.push_str(&format!("Scope: {scope}\n\n"));
    }

    if report.dependency_graph.is_empty() {
        md.push_str("No dependencies found.\n");
        return md;
    }

    // Collect unique connections: (source, target, protocol)
    let mut connections: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();

    for dep in &report.dependency_graph {
        match dep.api_type {
            ApiType::AsyncApi => {
                let kafka = "KAFKA".to_string();
                let protocol = "https".to_string();
                connections
                    .entry(dep.client.clone())
                    .or_default()
                    .insert((kafka.clone(), protocol.clone()));
                connections
                    .entry(dep.service.clone())
                    .or_default()
                    .insert((kafka, protocol));
            }
            ApiType::OpenApi | ApiType::Proto => {
                connections
                    .entry(dep.client.clone())
                    .or_default()
                    .insert((dep.service.clone(), "https".to_string()));
            }
        }
    }

    for (service, targets) in &connections {
        md.push_str(&format!("## Service: {}\n\n", service));
        md.push_str("| Target | Protocol | Port |\n");
        md.push_str("| --- | --- | --- |\n");
        for (target, protocol) in targets {
            md.push_str(&format!("| {} | {} | |\n", target, protocol));
        }
        md.push('\n');
    }

    md
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_dep(client: &str, service: &str, api_type: ApiType) -> DependencyInfo {
        DependencyInfo {
            client: client.to_string(),
            service: service.to_string(),
            api_type,
            version: SemVer::new(1, 2, 0),
            stability: Stability::Ga,
            path: "/test".to_string(),
            method: "GET".to_string(),
            deprecated: false,
        }
    }

    fn make_report(deps: Vec<DependencyInfo>) -> DependencyReport {
        DependencyReport {
            dependency_graph: deps,
            service_tags: std::collections::HashMap::new(),
            missing_endpoints: vec![],
            unused_endpoints: vec![],
            trunk_graph: vec![],
            harvested_subscriptions: vec![],
            peripheral_dependencies: vec![],
            systems: Default::default(),
            trunk_stale_before: None,
            scope_label: None,
        }
    }

    #[test]
    fn report_scope_parse_speaks_the_shared_selector_grammar() {
        assert!(matches!(
            ReportScope::parse(None).unwrap(),
            ReportScope::Dev
        ));
        assert!(matches!(
            ReportScope::parse(Some("dev")).unwrap(),
            ReportScope::Dev
        ));
        assert!(ReportScope::parse(Some("dev@2026-01-01T00:00:00Z")).is_err());
        assert!(matches!(
            ReportScope::parse(Some("main")).unwrap(),
            ReportScope::Main { at: None }
        ));
        match ReportScope::parse(Some("main@2026-01-01T02:00:00+02:00")).unwrap() {
            ReportScope::Main { at } => assert_eq!(at.as_deref(), Some("2026-01-01T00:00:00Z")),
            _ => panic!("expected the main scope"),
        }
        match ReportScope::parse(Some("rel-1")).unwrap() {
            ReportScope::Branch { name, at } => {
                assert_eq!(name, "rel-1");
                assert!(at.is_none());
            }
            _ => panic!("expected a branch scope"),
        }
    }

    #[tokio::test]
    async fn get_periphery_reads_the_set_of_the_scope() {
        use crate::application::mock_repo::MockRepo;
        let repo = MockRepo::new();
        let db = "resource:default/db".to_string();
        repo.reconcile_periphery(
            "svc",
            true,
            std::slice::from_ref(&db),
            "2026-01-01T00:00:00Z",
            Vec::new(),
        )
        .await
        .unwrap();
        repo.reconcile_periphery("svc", false, &[], "2026-02-01T00:00:00Z", Vec::new())
            .await
            .unwrap();
        let edge = PeripheralDependency {
            system: "svc".to_string(),
            resource: db,
        };
        assert!(
            get_periphery(&repo, &ReportScope::Dev)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            get_periphery(&repo, &ReportScope::Main { at: None })
                .await
                .unwrap(),
            vec![edge.clone()]
        );
        assert!(
            get_periphery(
                &repo,
                &ReportScope::Main {
                    at: Some("2025-12-31T00:00:00Z".to_string())
                }
            )
            .await
            .unwrap()
            .is_empty()
        );
        assert!(matches!(
            get_periphery(
                &repo,
                &ReportScope::Branch {
                    name: "nope".to_string(),
                    at: None
                }
            )
            .await,
            Err(AppError::NotFound(_))
        ));
    }

    fn harvested(client: &str, trunk: bool) -> HarvestedSubscription {
        HarvestedSubscription {
            client: client.to_string(),
            channel: "orders".to_string(),
            message_name: "Order".to_string(),
            owner: None,
            trunk,
        }
    }

    #[test]
    fn scope_harvested_subscriptions_keeps_what_the_scope_owns() {
        let all = vec![harvested("dev-only", false), harvested("trunk", true)];
        assert_eq!(
            scope_harvested_subscriptions(&ReportScope::Dev, all.clone()),
            all
        );
        assert_eq!(
            scope_harvested_subscriptions(&ReportScope::Main { at: None }, all.clone()),
            vec![harvested("trunk", true)]
        );
        let past = ReportScope::Main {
            at: Some("2026-01-01T00:00:00Z".to_string()),
        };
        assert!(scope_harvested_subscriptions(&past, all.clone()).is_empty());
        let branch = ReportScope::Branch {
            name: "rel".to_string(),
            at: None,
        };
        assert!(scope_harvested_subscriptions(&branch, all).is_empty());
    }

    #[test]
    fn last_provided_at_instant_never_answers_from_after_the_instant() {
        let updated = Some("2026-03-01T00:00:00Z");
        let recorded = Some("2026-01-01T00:00:00Z");
        assert_eq!(
            last_provided_at_instant(updated, recorded, None).as_deref(),
            Some("2026-03-01T00:00:00Z")
        );
        assert_eq!(
            last_provided_at_instant(updated, recorded, Some("2026-02-01T00:00:00Z")).as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
        assert_eq!(
            last_provided_at_instant(updated, None, Some("2026-02-01T00:00:00Z")),
            None
        );
        assert_eq!(last_provided_at_instant(None, None, None), None);
    }

    async fn seed_version(
        repo: &crate::application::mock_repo::MockRepo,
        service: &str,
        api_type: ApiType,
        version: &str,
        stability: Stability,
        now: &str,
    ) -> (i64, i64) {
        let sid = repo.ensure_service(service).await.unwrap();
        let id = repo
            .upsert_spec_version(crate::domain::ports::UpsertSpecVersion {
                outbox: Vec::new(),
                service_id: sid,
                api_type,
                version: version.parse().unwrap(),
                stability,
                content: "x",
                content_hash: "sha256:x",
                provided_by: "alice",
                expected_prior_hash: None,
                now_iso: now,
                endpoints: vec![],
            })
            .await
            .unwrap();
        (sid, id)
    }

    fn line(
        api_type: ApiType,
        version: &str,
        stability: Option<Stability>,
        last: Option<&str>,
    ) -> ScopeVersionLine {
        ScopeVersionLine {
            api_type,
            version: version.parse().unwrap(),
            stability,
            last_provided_at: last.map(str::to_string),
            channels: Vec::new(),
        }
    }

    #[tokio::test]
    async fn get_graph_scope_lists_dev_producers_with_their_newest_lines() {
        use crate::application::mock_repo::MockRepo;
        let repo = MockRepo::new();
        let t1 = "2026-01-01T00:00:00Z";
        seed_version(&repo, "a", ApiType::OpenApi, "1.0.0", Stability::Ga, t1).await;
        seed_version(&repo, "a", ApiType::OpenApi, "1.4.0", Stability::Ga, t1).await;
        seed_version(
            &repo,
            "a",
            ApiType::AsyncApi,
            "0.3.0",
            Stability::Snapshot,
            t1,
        )
        .await;
        let data = get_graph_scope(&repo, &ReportScope::Dev).await.unwrap();
        assert_eq!(
            data.producers,
            vec![ScopeProducer {
                name: "a".to_string(),
                versions: vec![
                    line(
                        ApiType::AsyncApi,
                        "0.3.0",
                        Some(Stability::Snapshot),
                        Some(t1)
                    ),
                    line(ApiType::OpenApi, "1.4.0", Some(Stability::Ga), Some(t1)),
                ],
            }]
        );
    }

    #[tokio::test]
    async fn get_graph_scope_reads_trunk_members_live_and_at_a_past_instant() {
        use crate::application::mock_repo::MockRepo;
        let repo = MockRepo::new();
        let (t1, t2, t3) = (
            "2026-01-01T00:00:00Z",
            "2026-02-01T00:00:00Z",
            "2026-03-01T00:00:00Z",
        );
        let (_, a1) = seed_version(&repo, "a", ApiType::OpenApi, "1.0.0", Stability::Ga, t1).await;
        repo.touch_spec_version_trunk(a1, t1).await.unwrap();
        let (_, a2) = seed_version(&repo, "a", ApiType::OpenApi, "2.0.0", Stability::Ga, t2).await;
        repo.touch_spec_version_trunk(a2, t2).await.unwrap();
        // A same-version trunk re-provide refreshes the record, not its start.
        repo.touch_spec_version_trunk(a2, t3).await.unwrap();
        // Never trunk-provided: not in main at all.
        seed_version(
            &repo,
            "dev-only",
            ApiType::OpenApi,
            "1.0.0",
            Stability::Ga,
            t1,
        )
        .await;

        let live = get_graph_scope(&repo, &ReportScope::Main { at: None })
            .await
            .unwrap();
        assert_eq!(
            live.producers,
            vec![ScopeProducer {
                name: "a".to_string(),
                versions: vec![line(
                    ApiType::OpenApi,
                    "2.0.0",
                    Some(Stability::Ga),
                    Some(t3)
                )],
            }]
        );

        let past = get_graph_scope(
            &repo,
            &ReportScope::Main {
                at: Some("2026-01-15T00:00:00Z".to_string()),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            past.producers,
            vec![ScopeProducer {
                name: "a".to_string(),
                versions: vec![line(
                    ApiType::OpenApi,
                    "1.0.0",
                    Some(Stability::Ga),
                    Some(t1)
                )],
            }]
        );
        // Between t2 and t3 the refresh at t3 is not yet known.
        let mid = get_graph_scope(
            &repo,
            &ReportScope::Main {
                at: Some("2026-02-15T00:00:00Z".to_string()),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            mid.producers[0].versions,
            vec![line(
                ApiType::OpenApi,
                "2.0.0",
                Some(Stability::Ga),
                Some(t2)
            )]
        );
        let before = get_graph_scope(
            &repo,
            &ReportScope::Main {
                at: Some("2025-12-01T00:00:00Z".to_string()),
            },
        )
        .await
        .unwrap();
        assert!(before.producers.is_empty());
    }

    #[tokio::test]
    async fn get_graph_scope_reads_branch_members_and_pins() {
        use crate::application::mock_repo::MockRepo;
        use crate::domain::ports::RecordTrunkPinParams;
        let repo = MockRepo::new();
        let (t1, t2) = ("2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z");
        let (pinned_sid, _) =
            seed_version(&repo, "pinned", ApiType::Proto, "3.1.0", Stability::Ga, t1).await;
        let (hotfix_sid, _) = seed_version(
            &repo,
            "hotfix",
            ApiType::OpenApi,
            "1.0.1",
            Stability::Ga,
            t2,
        )
        .await;
        let client = repo.ensure_client("consumer").await.unwrap();
        let branch = repo
            .insert_branch("rel", t1, "alice", "main", t1, Vec::new())
            .await
            .unwrap();
        repo.record_branch_pins(
            branch,
            vec![RecordTrunkPinParams {
                client_id: client,
                service_id: pinned_sid,
                api_type: ApiType::Proto,
                version: "3.1.0".parse().unwrap(),
                path: "/svc/Get",
                normalized_path: "/svc/Get",
                method: "RPC",
                now_iso: t1,
            }],
        )
        .await
        .unwrap();
        repo.record_branch_member_version(
            branch,
            hotfix_sid,
            ApiType::OpenApi,
            "1.0.1".parse().unwrap(),
            t2,
        )
        .await
        .unwrap();

        let live = get_graph_scope(
            &repo,
            &ReportScope::Branch {
                name: "rel".to_string(),
                at: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            live.producers,
            vec![
                ScopeProducer {
                    name: "hotfix".to_string(),
                    versions: vec![line(
                        ApiType::OpenApi,
                        "1.0.1",
                        Some(Stability::Ga),
                        Some(t2)
                    )],
                },
                ScopeProducer {
                    name: "pinned".to_string(),
                    versions: vec![line(ApiType::Proto, "3.1.0", Some(Stability::Ga), Some(t1))],
                },
            ]
        );
        let past = get_graph_scope(
            &repo,
            &ReportScope::Branch {
                name: "rel".to_string(),
                at: Some("2026-01-15T00:00:00Z".to_string()),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            past.producers
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["pinned"]
        );
        assert!(matches!(
            get_graph_scope(
                &repo,
                &ReportScope::Branch {
                    name: "nope".to_string(),
                    at: None
                }
            )
            .await,
            Err(AppError::NotFound(_))
        ));
    }

    #[test]
    fn empty_report() {
        let report = make_report(vec![]);
        let result = render_isolation_report(&report);
        assert!(result.contains("No dependencies found."));
        assert!(!result.contains("| Target |"));
    }

    #[test]
    fn markdown_report_names_version_and_stability() {
        let report = make_report(vec![make_dep(
            "order-service",
            "user-service",
            ApiType::OpenApi,
        )]);
        let result = render_report_markdown(&report);
        assert!(
            result.contains(
                "| order-service | user-service | OpenApi | 1.2.0 | ga | `/test` | `GET` |"
            )
        );
    }

    #[test]
    fn openapi_dependencies() {
        let report = make_report(vec![make_dep(
            "order-service",
            "user-service",
            ApiType::OpenApi,
        )]);
        let result = render_isolation_report(&report);
        assert!(result.contains("## Service: order-service"));
        assert!(result.contains("| user-service | https | |"));
    }

    #[test]
    fn asyncapi_dependencies_route_through_kafka() {
        let report = make_report(vec![make_dep(
            "order-service",
            "notification-service",
            ApiType::AsyncApi,
        )]);
        let result = render_isolation_report(&report);
        assert!(result.contains("## Service: order-service"));
        assert!(result.contains("## Service: notification-service"));
        assert!(result.contains("| KAFKA | https | |"));
        assert!(!result.contains("| notification-service |"));
        assert!(!result.contains("| order-service |"));
    }

    #[test]
    fn proto_dependencies() {
        let report = make_report(vec![make_dep("gateway", "auth-service", ApiType::Proto)]);
        let result = render_isolation_report(&report);
        assert!(result.contains("## Service: gateway"));
        assert!(result.contains("| auth-service | https | |"));
    }

    #[test]
    fn mixed_protocols() {
        let report = make_report(vec![
            make_dep("order-service", "user-service", ApiType::OpenApi),
            make_dep("order-service", "event-bus", ApiType::AsyncApi),
            make_dep("gateway", "auth-service", ApiType::Proto),
        ]);
        let result = render_isolation_report(&report);
        assert!(result.contains("## Service: order-service"));
        assert!(result.contains("| user-service | https | |"));
        assert!(result.contains("| KAFKA | https | |"));
        assert!(result.contains("## Service: gateway"));
        assert!(result.contains("| auth-service | https | |"));
        assert!(result.contains("## Service: event-bus"));
    }

    #[test]
    fn deduplication() {
        let report = make_report(vec![
            make_dep("order-service", "user-service", ApiType::OpenApi),
            make_dep("order-service", "user-service", ApiType::OpenApi),
        ]);
        let result = render_isolation_report(&report);
        let count = result.matches("| user-service | https | |").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn multiple_asyncapi_same_pair_deduplicated() {
        let report = make_report(vec![
            make_dep("svc-a", "svc-b", ApiType::AsyncApi),
            make_dep("svc-a", "svc-b", ApiType::AsyncApi),
        ]);
        let result = render_isolation_report(&report);
        let kafka_count = result.matches("| KAFKA | https | |").count();
        assert_eq!(kafka_count, 2);
    }
}
