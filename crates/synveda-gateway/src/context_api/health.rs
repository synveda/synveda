//! CPR-21 health is a projection of already authorized trace detail.

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
