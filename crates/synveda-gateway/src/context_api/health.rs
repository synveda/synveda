//! Health is a projection of already authorized trace detail.

use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use super::{ContextCandidateView, ContextFeedbackView, ContextSelectionView};
use crate::sessions::ContextRunView;
use synveda_types::{CaptureCandidateId, KnowledgeItemId, KnowledgeRevisionId};

/// Separate observable dimensions; there is deliberately no aggregate score.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextHealthDimension {
    Provenance,
    Freshness,
    Conflicts,
    Budget,
    Outcomes,
    Review,
}

/// Evidence availability, rather than a claim of universal context quality.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextHealthStatus {
    Observed,
    NeedsAttention,
    Unknown,
}

/// Supported existing workflow destinations; following one reauthorizes it.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextHealthAction {
    InspectRevision,
    InspectCandidate,
    InspectTrace,
    RequestCurrentContext,
    InspectSource,
    InspectConflict,
    InspectProposal,
    InspectOutcome,
    InspectEvaluation,
}

/// One bounded finding with at most one already authorized evidence example.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ContextHealthFindingView {
    pub dimension: ContextHealthDimension,
    pub status: ContextHealthStatus,
    /// Time of the trace/feedback observation, not a synchronization timestamp.
    pub observed_at: DateTime<Utc>,
    pub explanation: String,
    pub action: ContextHealthAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub knowledge_item_id: Option<KnowledgeItemId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub knowledge_revision_id: Option<KnowledgeRevisionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub capture_candidate_id: Option<CaptureCandidateId>,
    /// One exact currently authorized operational example.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<ContextHealthEvidence>,
}

/// Exact supported evidence destinations, without denied addresses or counts.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextHealthEvidence {
    /// Latest independently SourceRead-authorized configured-path observation.
    Source {
        /// Exact project.
        project_id: String,
        /// Exact configured source.
        source_id: String,
        /// Database clock of the latest complete snapshot, when one exists.
        observed_at: Option<DateTime<Utc>>,
        /// Exact covered path state, not inferred source inventory.
        path_state: Option<String>,
        /// Whether approved bytes differ from the observed current bytes.
        changed: bool,
        /// Whether the current live descriptor matches the observed definition.
        definition_matches: bool,
        /// Latest refresh request; separate from synchronization time.
        operation_id: Option<String>,
        /// Closed durable refresh lifecycle.
        operation_state: Option<String>,
    },
    /// Conflict disclosed only when every member remains visible.
    Conflict { conflict_set_id: String },
    /// Exact existing reviewed publication workflow.
    Proposal { proposal_id: String },
    /// Authenticated observation associated with the exact run/hash.
    Outcome {
        session_id: String,
        event_id: String,
        evidence_class: String,
        outcome: String,
    },
    /// Fixed-case qualification, without causal credit to included memory.
    Evaluation {
        attempt_id: String,
        candidate_id: String,
        qualified: Option<bool>,
    },
}

/// Distinct causal stages; multiple observations may require attention.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextPipelineStage {
    Representation,
    Eligibility,
    Retrieval,
    Assembly,
    DeliveryUse,
}

/// A production observation cannot claim knowledge of missing oracle evidence.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ContextStageObservationView {
    pub stage: ContextPipelineStage,
    pub status: ContextHealthStatus,
    pub explanation: String,
}

/// Current projection of a retained trace, without unrestricted store scans.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ContextHealthView {
    pub schema_version: u8,
    /// When the current authorized view was assessed.
    pub assessed_at: DateTime<Utc>,
    /// Exactly six independent dimensions, each with bounded evidence.
    pub findings: Vec<ContextHealthFindingView>,
    /// Exactly five stages; unknown is explicit.
    pub pipeline: Vec<ContextStageObservationView>,
}

impl ContextHealthFindingView {
    fn new(
        run: &ContextRunView,
        dimension: ContextHealthDimension,
        status: ContextHealthStatus,
        explanation: &str,
        action: ContextHealthAction,
        evidence: Option<&ContextCandidateView>,
    ) -> Self {
        Self {
            dimension,
            status,
            observed_at: run.created_at,
            explanation: explanation.to_owned(),
            action,
            knowledge_item_id: evidence.and_then(|entry| entry.knowledge_item_id),
            knowledge_revision_id: evidence.and_then(|entry| entry.knowledge_revision_id),
            capture_candidate_id: evidence.and_then(|entry| entry.capture_candidate_id),
            evidence: None,
        }
    }
}

pub(super) fn assess(
    run: &ContextRunView,
    candidates: &[ContextCandidateView],
    selections: &[ContextSelectionView],
    feedback: &[ContextFeedbackView],
) -> ContextHealthView {
    use ContextHealthAction as Action;
    use ContextHealthDimension as Dimension;
    use ContextHealthStatus as Status;

    let source = candidates.iter().find(|entry| !entry.sources.is_empty());
    let stale = candidates.iter().find(|entry| {
        entry.exclusion_reason.as_deref() == Some("stale")
            || entry
                .revision
                .as_ref()
                .is_some_and(|revision| revision.stale)
    });
    let conflict = candidates.iter().find(|entry| {
        entry.lifecycle_state.as_deref() == Some("transitional")
            || entry
                .reason_codes
                .iter()
                .any(|reason| reason == "contradiction_warning")
    });
    let budget = candidates
        .iter()
        .find(|entry| entry.exclusion_reason.as_deref() == Some("token_budget"));
    let excerpted = selections
        .iter()
        .any(|entry| entry.reason_codes.iter().any(|reason| reason == "excerpt"));
    let pending = candidates
        .iter()
        .find(|entry| entry.unreviewed_candidate.is_some());
    let correction = feedback.iter().find(|entry| {
        matches!(
            entry.feedback_type.as_str(),
            "unhelpful" | "caused_correction"
        )
    });
    let outcome = correction.or_else(|| feedback.first());

    let mut findings = vec![
        ContextHealthFindingView::new(
            run,
            Dimension::Provenance,
            if source.is_some() {
                Status::Observed
            } else {
                Status::Unknown
            },
            if source.is_some() {
                "Independently authorized provenance is visible. Completeness outside this view is unknown."
            } else {
                "Provenance completeness cannot be established from this retained authorized view."
            },
            if source.is_some() {
                Action::InspectRevision
            } else {
                Action::InspectTrace
            },
            source,
        ),
        ContextHealthFindingView::new(
            run,
            Dimension::Freshness,
            if stale.is_some() {
                Status::NeedsAttention
            } else {
                Status::Unknown
            },
            if stale.is_some() {
                "The trace observed evidence due for verification. Inspect its exact revision before reusing it. Source synchronization coverage remains unknown."
            } else {
                "This trace does not establish source synchronization coverage or external source freshness."
            },
            if stale.is_some() {
                Action::InspectRevision
            } else {
                Action::InspectTrace
            },
            stale,
        ),
        ContextHealthFindingView::new(
            run,
            Dimension::Conflicts,
            if conflict.is_some() {
                Status::NeedsAttention
            } else {
                Status::Unknown
            },
            if conflict.is_some() {
                "Visible evidence has a contradiction warning or a transitional state. Inspect the existing conflict review; recency alone cannot resolve it."
            } else {
                "No conflict is established by this visible trace. Applicability completeness remains unknown."
            },
            if conflict.is_some() {
                Action::InspectRevision
            } else {
                Action::InspectTrace
            },
            conflict,
        ),
        ContextHealthFindingView::new(
            run,
            Dimension::Budget,
            if budget.is_some() {
                Status::NeedsAttention
            } else if run.rendered.is_some() {
                Status::Observed
            } else {
                Status::Unknown
            },
            if budget.is_some() {
                "Eligible visible evidence did not fit the allowance. Inspect the excluded revision or request current context with a revised task and governed budget."
            } else if excerpted {
                "Exact source excerpts entered the offered context. Their spans can be inspected; semantic completeness and complete host-request fit are not established."
            } else if run.rendered.is_some() {
                "The offered Synveda contribution has retained budget accounting. Host history, model use and receiving-provider overhead remain unknown."
            } else {
                "Rendered budget and transformation evidence is unavailable in this authorized trace."
            },
            if budget.is_some() {
                Action::InspectRevision
            } else {
                Action::RequestCurrentContext
            },
            budget,
        ),
        ContextHealthFindingView::new(
            run,
            Dimension::Outcomes,
            if correction.is_some() {
                Status::NeedsAttention
            } else if outcome.is_some() {
                Status::Observed
            } else {
                Status::Unknown
            },
            if correction.is_some() {
                "An authenticated user reported unhelpful context or a correction. This is feedback, not independent proof of execution or the cause of task failure."
            } else if outcome.is_some() {
                "Authenticated feedback is retained. It does not establish independently verified execution, model use or causal task success."
            } else {
                "Acknowledged delivery, actual model use and independently verified task outcome are unknown."
            },
            Action::InspectTrace,
            None,
        ),
        ContextHealthFindingView::new(
            run,
            Dimension::Review,
            if pending.is_some() {
                Status::NeedsAttention
            } else {
                Status::Unknown
            },
            if pending.is_some() {
                "A visible capture proposal still needs its existing review workflow. Delivery cannot publish it. Cleanup completion is not established by this trace."
            } else {
                "Pending review and physical cleanup coverage cannot be established from this trace alone."
            },
            if pending.is_some() {
                Action::InspectCandidate
            } else {
                Action::InspectTrace
            },
            pending,
        ),
    ];
    if let Some(entry) = outcome {
        let finding = &mut findings[4];
        finding.observed_at = entry.created_at;
        if let Some(selection) = selections
            .iter()
            .find(|selection| selection.id == entry.context_selection_id)
        {
            finding.knowledge_item_id = selection.knowledge_item_id;
            finding.knowledge_revision_id = selection.knowledge_revision_id;
        }
    }
    let stage = |stage, status, explanation: &str| ContextStageObservationView {
        stage,
        status,
        explanation: explanation.to_owned(),
    };
    ContextHealthView {
        schema_version: 1,
        assessed_at: Utc::now(),
        findings,
        pipeline: vec![
            stage(
                ContextPipelineStage::Representation,
                Status::Unknown,
                "The planner trace cannot establish whether relevant source evidence was captured or represented. No oracle presence assertion is made.",
            ),
            stage(
                ContextPipelineStage::Eligibility,
                if stale.is_some() || pending.is_some() {
                    Status::NeedsAttention
                } else {
                    Status::Unknown
                },
                "Only visible lifecycle and proposal evidence is reported. Unavailable evidence is not counted or classified as a retrieval miss.",
            ),
            stage(
                ContextPipelineStage::Retrieval,
                if run.degraded.is_empty() {
                    Status::Observed
                } else {
                    Status::NeedsAttention
                },
                if run.degraded.is_empty() {
                    "The retained retrieval legs ran without a reported fallback. Recall against all eligible evidence is unknown."
                } else {
                    "The retained trace reports a degraded retrieval leg. Inspect the recorded fallback before attributing an omission to ranking."
                },
            ),
            stage(
                ContextPipelineStage::Assembly,
                if budget.is_some() {
                    Status::NeedsAttention
                } else if run.rendered.is_some() {
                    Status::Observed
                } else {
                    Status::Unknown
                },
                "Visible budget exclusions and exact transformations describe assembly. Whether a necessary qualifier was absent requires admissible task evidence or a separate offline oracle.",
            ),
            stage(
                ContextPipelineStage::DeliveryUse,
                if correction.is_some() {
                    Status::NeedsAttention
                } else {
                    Status::Unknown
                },
                "A ContextRun records offered context; feedback is a user assertion. Acknowledged host delivery, actual use and verified outcome require separate evidence.",
            ),
        ],
    }
}

/// Enrich after the existing trace projection. Every query starts from a
/// currently visible exact address and is bounded independently of tenant size.
pub(super) async fn enrich(
    state: &crate::app::AppState,
    tx: &mut sqlx::PgConnection,
    tenant: synveda_types::TenantId,
    run: &ContextRunView,
    candidates: &[ContextCandidateView],
    health: &mut ContextHealthView,
) -> synveda_types::Result<()> {
    use synveda_policy::Action;
    use synveda_types::{Error, ProjectId, SessionEventId, SessionId};
    if !matches!(run.trace_retention_mode.as_str(), "full" | "redacted") {
        return Ok(());
    }
    let mut sync = false;
    let mut conflict = false;
    let mut evaluation = false;
    for candidate in candidates.iter().take(32) {
        if let Some(revision) = candidate.knowledge_revision_id {
            if !sync {
                let mut signals =
                    synveda_store::knowledge_freshness::source_observations(tx, tenant, revision)
                        .await?;
                signals.truncate(32);
                signals.sort_by_key(|s| (!s.changed, s.source_id.clone()));
                for source in signals {
                    let project = ProjectId::from_uuid(source.project_id);
                    match crate::source_reconciliation::authorize_project(
                        state,
                        tx,
                        tenant,
                        project,
                        Action::SourceRead,
                    )
                    .await
                    {
                        Ok(_) => {}
                        Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => continue,
                        Err(e) => return Err(e),
                    }
                    let (_, _, descriptor) = match crate::source_reconciliation::configured(
                        tx,
                        tenant,
                        project,
                        &source.source_id,
                    )
                    .await
                    {
                        Ok(value) => value,
                        Err(Error::NotFound { .. } | Error::PolicyDenied { .. }) => continue,
                        Err(e) => return Err(e),
                    };
                    let definition_matches = source.descriptor_hash.as_deref()
                        == Some(
                            crate::source_reconciliation::descriptor_hash(&descriptor)?.as_str(),
                        );
                    let coverage_known =
                        definition_matches && descriptor.enabled && source.path_state.is_some();
                    let adverse = definition_matches
                        && (source.changed
                            || source
                                .operation_state
                                .as_deref()
                                .is_some_and(|s| matches!(s, "failed" | "blocked" | "cancelled")));
                    if let Some(finding) = health
                        .findings
                        .iter_mut()
                        .find(|f| matches!(f.dimension, ContextHealthDimension::Freshness))
                    {
                        if !coverage_known
                            && !matches!(finding.status, ContextHealthStatus::NeedsAttention)
                        {
                            finding.status = ContextHealthStatus::Unknown
                        }
                        if definition_matches
                            && (source.changed
                                || source.operation_state.as_deref().is_some_and(|s| {
                                    matches!(s, "failed" | "blocked" | "cancelled")
                                }))
                        {
                            finding.status = ContextHealthStatus::NeedsAttention
                        } else if matches!(finding.status, ContextHealthStatus::Unknown)
                            && coverage_known
                            && source.observed_at.is_some()
                        {
                            finding.status = ContextHealthStatus::Observed
                        }
                        finding.observed_at = source.observed_at.unwrap_or(health.assessed_at);
                        finding.explanation = format!(
                            "{} The exact configured source has {} complete observation; refresh state is recorded separately.",
                            finding.explanation,
                            if source.observed_at.is_some() {
                                "a retained"
                            } else {
                                "no retained"
                            }
                        );
                        if !definition_matches {
                            finding.explanation="The retained source snapshot predates the current definition. Inspect its enabled state and coverage; current synchronization is unknown.".to_owned();
                        } else if !coverage_known {
                            finding.explanation="The complete current source snapshot does not establish coverage of this retained path. Inspect the source's enabled state and allowed paths; synchronization for this item is unknown.".to_owned();
                        }
                        finding.action = ContextHealthAction::InspectSource;
                        finding.evidence = Some(ContextHealthEvidence::Source {
                            project_id: project.to_string(),
                            source_id: source.source_id,
                            observed_at: source.observed_at,
                            path_state: if definition_matches {
                                source.path_state
                            } else {
                                None
                            },
                            changed: definition_matches && source.changed,
                            definition_matches,
                            operation_id: source.operation_id.map(|id| id.to_string()),
                            operation_state: source.operation_state,
                        });
                    }
                    if adverse {
                        sync = true;
                        break;
                    }
                }
            }
            if !conflict {
                for id in synveda_store::knowledge_conflicts::for_revision(tx, tenant, revision)
                    .await?
                    .into_iter()
                    .take(32)
                {
                    let Some(set) =
                        synveda_store::knowledge_conflicts::get(&mut *tx, tenant, id).await?
                    else {
                        continue;
                    };
                    let Some(set) =
                        crate::knowledge_conflicts::visible_set(state, tx, tenant, set).await?
                    else {
                        continue;
                    };
                    if let Some(finding) = health
                        .findings
                        .iter_mut()
                        .find(|f| matches!(f.dimension, ContextHealthDimension::Conflicts))
                    {
                        finding.status =
                            if matches!(finding.status, ContextHealthStatus::NeedsAttention) {
                                ContextHealthStatus::NeedsAttention
                            } else if matches!(set.status.as_str(), "resolved" | "dismissed") {
                                ContextHealthStatus::Observed
                            } else {
                                ContextHealthStatus::NeedsAttention
                            };
                        finding.observed_at = set.updated_at;
                        finding.explanation="An exact durable conflict is independently visible in full; inspect its current applicability and review state.".to_owned();
                        finding.action = ContextHealthAction::InspectConflict;
                        finding.evidence = Some(ContextHealthEvidence::Conflict {
                            conflict_set_id: id.to_string(),
                        });
                    }
                    if !matches!(set.status.as_str(), "resolved" | "dismissed") {
                        conflict = true;
                        break;
                    }
                }
            }
        }
        if let Some(candidate) = &candidate.unreviewed_candidate {
            if let Some(change) = candidate.resulting_change_id
                && let Some(finding) = health
                    .findings
                    .iter_mut()
                    .find(|f| matches!(f.dimension, ContextHealthDimension::Review))
            {
                finding.action = ContextHealthAction::InspectProposal;
                finding.evidence = Some(ContextHealthEvidence::Proposal {
                    proposal_id: change.to_string(),
                });
            }
            if !evaluation
                && let Some(attempt) =
                    synveda_store::procedure_learning::attempts(tx, tenant, candidate.id)
                        .await?
                        .into_iter()
                        .next()
            {
                crate::capture::load_candidate(
                    state,
                    tx,
                    tenant,
                    candidate.id,
                    Action::SessionRead,
                )
                .await?;
                let result =
                    synveda_store::procedure_learning::result(tx, tenant, attempt.id).await?;
                if let Some(finding) = health
                    .findings
                    .iter_mut()
                    .find(|f| matches!(f.dimension, ContextHealthDimension::Review))
                {
                    finding.action = ContextHealthAction::InspectEvaluation;
                    finding.evidence = Some(ContextHealthEvidence::Evaluation {
                        attempt_id: attempt.id.to_string(),
                        candidate_id: candidate.id.to_string(),
                        qualified: result.map(|r| r.qualified),
                    });
                    finding.explanation="A Procedure candidate has a frozen independent evaluation. Qualification and publication review remain separate gates.".to_owned();
                }
                evaluation = true;
            }
        }
    }
    let mut outcomes = synveda_store::procedure_learning::for_context(tx, tenant, run.id).await?;
    outcomes.truncate(32);
    outcomes.sort_by_key(|o| (o.outcome != "failed", std::cmp::Reverse(o.received_at)));
    for outcome in outcomes {
        let session = SessionId::from_uuid(outcome.session_id);
        match crate::sessions::load(state, tx, tenant, session, Action::SessionDiagnostics).await {
            Ok(_) => {}
            Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => continue,
            Err(e) => return Err(e),
        }
        let Some(event) = synveda_store::sessions::event(
            &mut *tx,
            tenant,
            session,
            SessionEventId::from_uuid(outcome.event_id),
        )
        .await?
        else {
            continue;
        };
        match crate::procedure_learning::authorize_event(state, tx, tenant, &event).await {
            Ok(()) => {}
            Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => continue,
            Err(e) => return Err(e),
        }
        if outcome.evidence_class != "authenticated_runner" {
            continue;
        }
        let status = match outcome.outcome.as_str() {
            "failed" => ContextHealthStatus::NeedsAttention,
            "succeeded" => ContextHealthStatus::Observed,
            _ => ContextHealthStatus::Unknown,
        };
        let explanation = "An authenticated runner reported an outcome for this exact delivered block. The cause and causal credit to included memory remain unknown.";
        if let Some(finding) = health
            .findings
            .iter_mut()
            .find(|f| matches!(f.dimension, ContextHealthDimension::Outcomes))
        {
            if !matches!(finding.status, ContextHealthStatus::NeedsAttention) {
                finding.status = status;
            }
            finding.observed_at = outcome.received_at;
            finding.explanation = explanation.to_owned();
            finding.action = ContextHealthAction::InspectOutcome;
            finding.evidence = Some(ContextHealthEvidence::Outcome {
                session_id: session.to_string(),
                event_id: outcome.event_id.to_string(),
                evidence_class: outcome.evidence_class,
                outcome: outcome.outcome,
            });
        }
        if let Some(stage) = health
            .pipeline
            .iter_mut()
            .find(|s| matches!(s.stage, ContextPipelineStage::DeliveryUse))
        {
            if !matches!(stage.status, ContextHealthStatus::NeedsAttention) {
                stage.status = status;
            }
            stage.explanation = explanation.to_owned()
        }
        break;
    }
    Ok(())
}
