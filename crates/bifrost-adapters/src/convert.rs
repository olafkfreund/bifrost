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

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Detect languages + build tools from the pipeline YAML (source + the Importer's
/// converted output) to populate the grounded request's `repo_context` (plan
/// §5.3). A pragmatic v1 — the pipeline is the signal we have without cloning the
/// repo; a clone-based detector can layer on later. Returns `languages: unknown`
/// when nothing matches, preserving prior behaviour.
fn labels_matching<'a>(table: &[(&'a str, &'a [&'a str])], hay: &str) -> Vec<&'a str> {
    table
        .iter()
        .filter(|(_, needles)| needles.iter().any(|n| hay.contains(*n)))
        .map(|(label, _)| *label)
        .collect()
}

fn detect_repo_context(source_yaml: &str, converted_yaml: &str) -> String {
    let hay = format!("{source_yaml}\n{converted_yaml}").to_ascii_lowercase();

    let languages: &[(&str, &[&str])] = &[
        ("C#/.NET", &["dotnet", ".csproj", "nuget", "msbuild"]),
        ("Java", &["gradle", "mvn ", "maven", "pom.xml", "gradlew"]),
        (
            "JavaScript/TypeScript",
            &["npm ", "node ", "yarn", "package.json", "tsc"],
        ),
        (
            "Python",
            &["python", "pip ", "pytest", "requirements.txt", "poetry"],
        ),
        ("Go", &["go build", "go test", "go.mod", "golang"]),
        ("Rust", &["cargo ", "rustup"]),
    ];
    let build_tools: &[(&str, &[&str])] = &[
        ("dotnet", &["dotnet"]),
        ("gradle", &["gradle", "gradlew"]),
        ("maven", &["mvn ", "maven", "pom.xml"]),
        ("npm", &["npm ", "package.json"]),
        ("docker", &["docker build", "dockerfile", "docker-compose"]),
        ("terraform", &["terraform"]),
    ];

    let langs = labels_matching(languages, &hay);
    let tools = labels_matching(build_tools, &hay);
    let mut parts = Vec::new();
    if !langs.is_empty() {
        parts.push(format!("languages: {}", langs.join(", ")));
    }
    if !tools.is_empty() {
        parts.push(format!("build tools: {}", tools.join(", ")));
    }
    if parts.is_empty() {
        "languages: unknown".to_string()
    } else {
        parts.join("; ")
    }
}

/// Extract the source snippet for `construct` from the ADO definition: the line
/// naming the construct plus its indented block (e.g. the whole `- task: …`
/// step or the `matrix:` mapping). Falls back to the construct id when the
/// source is unavailable or the construct isn't found, so grounding never
/// degrades below today's behaviour.
fn source_snippet_for(source_yaml: &str, construct: &str) -> String {
    if source_yaml.trim().is_empty() {
        return construct.to_string();
    }
    let lines: Vec<&str> = source_yaml.lines().collect();
    // Match the full construct, else its last dotted segment
    // (e.g. `strategy.matrix` → `matrix`).
    let token = construct.rsplit('.').next().unwrap_or(construct);
    let Some(start) = lines
        .iter()
        .position(|l| l.contains(construct) || l.contains(token))
    else {
        return construct.to_string();
    };

    let base = indent_of(lines[start]);
    let mut end = start + 1;
    while end < lines.len() {
        let line = lines[end];
        if line.trim().is_empty() || indent_of(line) > base {
            end += 1;
        } else {
            break;
        }
    }
    lines[start..end].join("\n").trim_end().to_string()
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

    // Detect languages/build tools from the pipeline itself and combine with any
    // caller-supplied context (e.g. a future repo-clone-based detector).
    let detected = detect_repo_context(&dry.source_yaml, &dry.converted_yaml);
    let context = if repo_context.trim().is_empty() {
        detected
    } else {
        format!("{}; {}", repo_context.trim(), detected)
    };

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
            // Ground the model in the actual ADO source for this construct (its
            // line + indented block), not just the construct id.
            source_snippet: source_snippet_for(&dry.source_yaml, &gap.construct),
            converted_yaml: dry.converted_yaml.clone(),
            importer_message: gap.detail.clone(),
            repo_context: context.clone(),
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

    const SOURCE: &str = "trigger:\n  branches:\n    include:\n      - main\n\nstrategy:\n  matrix:\n    linux:\n      imageName: ubuntu-latest\n    windows:\n      imageName: windows-latest\n\nsteps:\n  - task: DownloadSecureFile@1\n    name: signingCert\n    inputs:\n      secureFile: code-signing.pfx\n\n  - script: dotnet build\n    displayName: Build\n";

    #[test]
    fn snippet_extracts_the_task_block_with_its_inputs() {
        let s = source_snippet_for(SOURCE, "DownloadSecureFile@1");
        assert!(s.contains("- task: DownloadSecureFile@1"));
        assert!(s.contains("name: signingCert"));
        assert!(s.contains("secureFile: code-signing.pfx"));
        // Stops at the next sibling step (the `- script:` block is excluded).
        assert!(!s.contains("dotnet build"));
    }

    #[test]
    fn snippet_matches_a_dotted_construct_by_last_segment() {
        let s = source_snippet_for(SOURCE, "strategy.matrix");
        assert!(s.contains("matrix:"));
        assert!(s.contains("imageName: ubuntu-latest"));
        assert!(s.contains("windows:"));
    }

    #[test]
    fn snippet_falls_back_to_the_construct_when_unavailable() {
        assert_eq!(source_snippet_for("", "Foo@1"), "Foo@1");
        assert_eq!(source_snippet_for(SOURCE, "NotPresent@9"), "NotPresent@9");
    }

    #[test]
    fn detects_dotnet_from_the_pipeline() {
        let ctx = detect_repo_context(SOURCE, "");
        assert!(ctx.contains("C#/.NET"), "got: {ctx}");
        assert!(ctx.contains("build tools: dotnet"), "got: {ctx}");
    }

    #[test]
    fn detects_multiple_languages_and_tools() {
        let yaml = "steps:\n  - script: ./gradlew build\n  - script: npm ci && npm test\n  - script: docker build .\n";
        let ctx = detect_repo_context(yaml, "");
        assert!(ctx.contains("Java"), "got: {ctx}");
        assert!(ctx.contains("JavaScript/TypeScript"), "got: {ctx}");
        assert!(ctx.contains("gradle"));
        assert!(ctx.contains("docker"));
    }

    #[test]
    fn unknown_when_nothing_detected() {
        assert_eq!(
            detect_repo_context("trigger:\n  - main\n", ""),
            "languages: unknown"
        );
    }
}
