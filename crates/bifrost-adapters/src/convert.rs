//! The per-pipeline conversion loop (plan §5).
//!
//! Ties the pieces together: dry-run a pipeline, route its non-manual gaps
//! through the [`Router`] for grounded gap-fill, assemble the augmented workflow
//! (Importer baseline + fills, with provenance), compute deterministic risk, and
//! emit a [`Proposal`] plus its manual-task [`Runbook`].
//!
//! The split between LLM work and human work is [`gap_is_manual`]: manual gaps
//! become runbook items; first-party unsupported steps and partial constructs
//! are filled by the model. The two never overlap. Risk is computed by the
//! deterministic engine, never the LLM (hard rule).
//!
//! Both collaborators are traits ([`Importer`], [`LlmProvider`] via [`Router`]),
//! so the whole loop runs and is tested offline with mocks — no Docker, no
//! network, no API key.

use bifrost_core::{
    assemble_workflow, assess, gap_is_manual, signals_from_dry_run, Classification, Gap, GapFill,
    GapKind, Proposal, Runbook,
};
use bifrost_llm::{GapFillRequest, LlmError, Router, TaskClass, GAP_FILL_PROMPT_ID};

use crate::importer::{Importer, ImporterError};

/// The output of converting one pipeline: the reviewable proposal and the
/// human checklist that accompanies it.
#[derive(Debug, Clone)]
pub struct ConversionOutcome {
    pub proposal: Proposal,
    pub runbook: Runbook,
}

/// Errors the conversion loop surfaces, wrapping its collaborators.
#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    #[error(transparent)]
    Importer(#[from] ImporterError),
    #[error(transparent)]
    Llm(#[from] LlmError),
}

/// Convert a single pipeline into a [`Proposal`] (+ [`Runbook`]).
///
/// Steps: dry-run → split gaps (`gap_is_manual`) → gap-fill the non-manual gaps
/// via `router` → [`assemble_workflow`] → deterministic [`assess`] →
/// [`Proposal::new`] in `Draft`. `repo_context` (languages / build tools) is
/// passed into each grounded request.
pub async fn convert_pipeline(
    importer: &dyn Importer,
    router: &Router<'_>,
    pipeline_id: &str,
    proposal_id: &str,
    classification: Classification,
    repo_context: &str,
) -> Result<ConversionOutcome, ConversionError> {
    let dry = importer.dry_run(pipeline_id).await?;

    // Human work → the runbook.
    let runbook = Runbook::from_gaps(&dry.gaps);

    // Model work → grounded gap-fill via the router, one request per gap.
    let mut fills: Vec<GapFill> = Vec::new();
    let mut rationales: Vec<String> = Vec::new();
    let mut risk_flags: Vec<String> = Vec::new();
    let mut verify_steps: Vec<String> = Vec::new();
    let mut confidences: Vec<f64> = Vec::new();

    for gap in dry.gaps.iter().filter(|g| !gap_is_manual(g)) {
        let provider = router.route(task_class_for(gap))?;
        let request = GapFillRequest {
            gap: gap.clone(),
            // The construct id is the best grounding available without the ADO
            // source; richer snippets arrive when wired to the SourceAdapter.
            source_snippet: gap.construct.clone(),
            converted_yaml: dry.converted_yaml.clone(),
            importer_message: gap.detail.clone(),
            repo_context: repo_context.to_string(),
        };
        let response = provider.fill_gap(&request).await?;

        fills.push(GapFill {
            construct: gap.construct.clone(),
            prompt_id: GAP_FILL_PROMPT_ID.to_string(),
            yaml: response.proposed_yaml,
        });
        rationales.push(format!("{}: {}", gap.construct, response.rationale));
        risk_flags.extend(response.risk_flags);
        verify_steps.extend(response.verify_steps);
        confidences.push(response.confidence);
    }

    let proposed_yaml = assemble_workflow(&dry.converted_yaml, &fills);

    // Deterministic risk — derived from the gaps, not the model.
    let assessment = assess(&signals_from_dry_run(&dry, classification));

    let confidence = if confidences.is_empty() {
        1.0 // nothing needed filling
    } else {
        confidences.iter().sum::<f64>() / confidences.len() as f64
    };

    let proposal = Proposal::new(
        proposal_id,
        pipeline_id,
        dry.source_yaml.clone(),
        proposed_yaml,
        rationales.join("\n"),
        risk_flags,
        verify_steps,
        GAP_FILL_PROMPT_ID,
        confidence,
        &assessment,
    );

    Ok(ConversionOutcome { proposal, runbook })
}

/// Route a fillable gap by intent: partial constructs are mechanical reshaping
/// (bulk); unsupported first-party steps need genuine reasoning.
fn task_class_for(gap: &Gap) -> TaskClass {
    match gap.kind {
        GapKind::PartialConstruct => TaskClass::Bulk,
        _ => TaskClass::HardReasoning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importer::MockImporter;
    use bifrost_core::ProposalStatus;
    use bifrost_llm::{MockLlmProvider, RoutingPolicy};

    /// Route everything to the local mock provider.
    fn mock_policy() -> RoutingPolicy {
        RoutingPolicy {
            bulk: vec!["mock".into()],
            hard: vec!["mock".into()],
            docs: vec!["mock".into()],
        }
    }

    #[tokio::test]
    async fn converts_a_pipeline_into_a_draft_proposal_and_runbook() {
        let importer = MockImporter;
        let mock = MockLlmProvider;
        let router = Router::new(vec![&mock], /* air_gap */ false).with_policy(mock_policy());

        let outcome = convert_pipeline(
            &importer,
            &router,
            "SARC-main",
            "prop-1",
            Classification::Yaml,
            "languages: dotnet",
        )
        .await
        .expect("conversion succeeds");

        let p = outcome.proposal;
        // A fresh proposal starts in Draft.
        assert_eq!(p.status, ProposalStatus::Draft);
        assert_eq!(p.pipeline_id, "SARC-main");

        // Assembled YAML keeps the Importer baseline AND the provenance-tagged
        // gap-fills for the two fillable gaps (DownloadSecureFile@1, strategy.matrix).
        assert!(p.proposed_yaml.contains("Importer-converted baseline step"));
        assert!(p.proposed_yaml.contains("REVIEW BEFORE USE"));
        assert!(p
            .proposed_yaml
            .contains("# bifrost-gap-fill: DownloadSecureFile@1 (prompt: gap-fill.v1)"));
        assert!(p
            .proposed_yaml
            .contains("# bifrost-gap-fill: strategy.matrix (prompt: gap-fill.v1)"));

        // The source ADO definition is carried through for the review diff.
        assert!(p.source_yaml.contains("DownloadSecureFile@1"));
        assert!(p.source_yaml.contains("strategy:"));

        // Risk is the deterministic engine's verdict for this fixture (amber).
        assert_eq!(p.risk_band, bifrost_core::RiskBand::Amber);
        assert_eq!(p.prompt_id, "gap-fill.v1");

        // Manual gaps went to the runbook (secret, service-connection,
        // environment + the namespaced custom task), not the gap-fills.
        assert_eq!(outcome.runbook.len(), 4);
        assert!(!p.proposed_yaml.contains("AZURE_CLIENT_SECRET"));
    }

    #[tokio::test]
    async fn air_gap_conversion_uses_only_the_local_provider() {
        // The mock is local, so an air-gap router still resolves it.
        let importer = MockImporter;
        let mock = MockLlmProvider;
        let router = Router::new(vec![&mock], /* air_gap */ true).with_policy(mock_policy());

        let outcome = convert_pipeline(
            &importer,
            &router,
            "SARC-main",
            "prop-2",
            Classification::Yaml,
            "languages: dotnet",
        )
        .await
        .expect("air-gap conversion succeeds with a local provider");
        assert_eq!(outcome.proposal.status, ProposalStatus::Draft);
    }
}
