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
    assemble_workflow, assess, gap_is_manual, signals_from_dry_run, ChecklistCategory,
    ChecklistItem, Classification, Gap, GapFill, GapKind, Proposal, RiskAssessment, Runbook,
};
use bifrost_llm::{
    CostLedger, GapFillRequest, JobCost, LlmError, LlmProvider, MeteredProvider, RateLimiter,
    Router, TaskClass, TokenBudget, GAP_FILL_PROMPT_ID,
};

use crate::importer::{Importer, ImporterError};

/// The output of converting one pipeline: the reviewable proposal and the
/// human checklist that accompanies it.
#[derive(Debug, Clone)]
pub struct ConversionOutcome {
    pub proposal: Proposal,
    pub runbook: Runbook,
    /// Per-job LLM token/cost accounting (#104). Empty for classic pipelines
    /// (no gap-fill ran) and air-gap-free local runs (tokens counted, cost $0).
    pub cost: JobCost,
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

    // Deterministic risk — derived from the gaps + classification, not the model.
    let assessment = assess(&signals_from_dry_run(&dry, classification));

    // Classic/designer pipelines have no YAML source to convert (plan §10, the
    // hard tail): surface a manual-rework scaffold instead of auto gap-fill.
    if classification == Classification::Classic {
        return Ok(classic_outcome(proposal_id, pipeline_id, &dry, &assessment));
    }

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
    // Model provenance (#159): which LLM provider(s) produced the gap-fills, so an
    // auditor can prove e.g. only a local model touched this pipeline in air-gap.
    let mut providers_used: Vec<String> = Vec::new();

    // Per-job token/cost accounting (#104). The ledger accumulates across every
    // gap; an optional token budget and frontier concurrency cap come from the
    // environment (opt-in, defaults are unlimited / unthrottled).
    let ledger = CostLedger::default();
    let budget = job_token_budget();
    let limiter = llm_rate_limiter();

    for gap in dry.gaps.iter().filter(|g| !gap_is_manual(g)) {
        let provider = router.route(task_class_for(gap))?;
        providers_used.push(provider.name().to_string());
        let request = GapFillRequest {
            gap: gap.clone(),
            // Ground the model in the actual ADO source for this construct (its
            // line + indented block), not just the construct id.
            source_snippet: source_snippet_for(&dry.source_yaml, &gap.construct),
            converted_yaml: dry.converted_yaml.clone(),
            importer_message: gap.detail.clone(),
            repo_context: context.clone(),
        };
        // Meter the routed provider: budget-gate, rate-limit frontiers, and record
        // usage into the shared ledger. The wrapper is itself an LlmProvider.
        let mut metered = MeteredProvider::new(provider, ledger.clone()).with_budget(budget);
        if let Some(limiter) = &limiter {
            metered = metered.with_rate_limit(limiter.clone());
        }
        let response = metered.fill_gap(&request).await?;

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

    let confidence = if confidences.is_empty() {
        1.0 // nothing needed filling
    } else {
        confidences.iter().sum::<f64>() / confidences.len() as f64
    };

    let mut proposal = Proposal::new(
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
    providers_used.sort_unstable();
    providers_used.dedup();
    proposal.llm_providers = providers_used;

    let cost = JobCost::from_ledger(&ledger);
    Ok(ConversionOutcome {
        proposal,
        runbook,
        cost,
    })
}

/// The per-job token budget from `BIFROST_TOKEN_BUDGET` (total tokens); unlimited
/// when unset, empty, zero, or unparseable.
fn job_token_budget() -> TokenBudget {
    match std::env::var("BIFROST_TOKEN_BUDGET")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
    {
        Some(n) if n > 0 => TokenBudget::tokens(n),
        _ => TokenBudget::unlimited(),
    }
}

/// A frontier concurrency cap from `BIFROST_LLM_MAX_CONCURRENCY`; `None` (no cap)
/// when unset, zero, or unparseable.
fn llm_rate_limiter() -> Option<RateLimiter> {
    std::env::var("BIFROST_LLM_MAX_CONCURRENCY")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .map(RateLimiter::concurrency)
}

const CLASSIC_BANNER: &str = "\
# ──────────────────────────────────────────────────────────────────────
# CLASSIC (designer) PIPELINE — MANUAL MIGRATION REQUIRED
# This Azure DevOps pipeline is defined in the designer (no YAML source),
# so it cannot be auto-converted. Recreate each task as a workflow step;
# see the runbook for the manual checklist.
# ──────────────────────────────────────────────────────────────────────";

/// Build the manual-rework outcome for a classic/designer pipeline: no LLM
/// gap-fill (there is no source to ground from), a clearly-flagged scaffold, and
/// a runbook led by a "re-author in YAML" item. Risk stays the deterministic
/// engine's verdict (classic defaults Amber/Red).
fn classic_outcome(
    proposal_id: &str,
    pipeline_id: &str,
    dry: &bifrost_core::DryRunResult,
    assessment: &RiskAssessment,
) -> ConversionOutcome {
    let proposed_yaml = if dry.converted_yaml.trim().is_empty() {
        CLASSIC_BANNER.to_string()
    } else {
        format!("{CLASSIC_BANNER}\n\n{}", dry.converted_yaml)
    };

    let mut runbook = Runbook::from_gaps(&dry.gaps);
    runbook.items.insert(
        0,
        ChecklistItem {
            category: ChecklistCategory::ReplacementAction,
            title: "Re-author this classic pipeline in YAML".into(),
            construct: "classic-pipeline".into(),
            detail:
                "Designer-defined pipeline has no YAML source — the Importer cannot convert it; \
                     recreate each task as a GitHub Actions workflow step."
                    .into(),
            required: true,
            done: false,
        },
    );

    let proposal = Proposal::new(
        proposal_id,
        pipeline_id,
        dry.source_yaml.clone(),
        proposed_yaml,
        "Classic/designer pipeline: no YAML source to convert — this is a manual-rework scaffold, \
         not an auto-conversion."
            .to_string(),
        vec![
            "Classic pipeline: every step must be re-authored by hand; designer pipelines are \
              not auto-convertible."
                .to_string(),
        ],
        vec![
            "Recreate each designer task as a workflow step".to_string(),
            "Run the workflow in a sandbox and compare to the classic pipeline".to_string(),
        ],
        // No gap-fill prompt ran — mark provenance accordingly.
        "classic-manual",
        0.0,
        assessment,
    );

    // No gap-fill runs for classic pipelines, so there is nothing to meter.
    ConversionOutcome {
        proposal,
        runbook,
        cost: JobCost::default(),
    }
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
    use bifrost_llm::{GapFillResponse, LlmProvider, MockLlmProvider, RoutingPolicy};

    /// Route everything to the local mock provider.
    fn mock_policy() -> RoutingPolicy {
        RoutingPolicy {
            bulk: vec!["mock".into()],
            hard: vec!["mock".into()],
            docs: vec!["mock".into()],
        }
    }

    /// A non-local "frontier" provider that **panics if its network call is ever
    /// reached** — a tripwire for egress (#102). In air-gap mode the Router must
    /// never route to it, so it must never fire.
    struct EgressTripwire;

    #[async_trait::async_trait]
    impl LlmProvider for EgressTripwire {
        fn name(&self) -> &str {
            "frontier-tripwire"
        }
        fn is_local(&self) -> bool {
            false
        }
        async fn fill_gap(&self, _req: &GapFillRequest) -> Result<GapFillResponse, LlmError> {
            panic!("EGRESS: a non-local provider was called in air-gap mode");
        }
    }

    #[tokio::test]
    async fn classic_pipeline_becomes_a_manual_rework_proposal_without_gap_fill() {
        let importer = MockImporter;
        let mock = MockLlmProvider;
        let router = Router::new(vec![&mock], /* air_gap */ false).with_policy(mock_policy());

        let outcome = convert_pipeline(
            &importer,
            &router,
            "classic-deploy",
            "prop-classic",
            Classification::Classic,
            "",
        )
        .await
        .expect("classic conversion succeeds");

        let p = outcome.proposal;
        assert_eq!(p.status, ProposalStatus::Draft);
        // A manual-rework scaffold — no LLM gap-fill ran.
        assert!(p.proposed_yaml.contains("MANUAL MIGRATION REQUIRED"));
        assert!(!p.proposed_yaml.contains("bifrost-gap-fill"));
        assert_eq!(p.prompt_id, "classic-manual");
        assert_eq!(p.confidence, 0.0);
        // The runbook is led by the "re-author in YAML" item.
        assert_eq!(
            outcome.runbook.items[0].category,
            bifrost_core::ChecklistCategory::ReplacementAction
        );
        assert!(outcome.runbook.items[0].title.contains("Re-author"));
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

        // Per-job token/cost accounting (#104): the two fillable gaps were metered
        // through the local mock provider — tokens counted, cost $0 (local is free).
        assert_eq!(outcome.cost.calls, 2, "one metered call per fillable gap");
        assert!(
            outcome.cost.total_tokens > 0,
            "tokens are estimated + recorded"
        );
        assert_eq!(outcome.cost.total_cost_usd, 0.0, "local provider is free");
        assert_eq!(outcome.cost.by_provider.len(), 1);
        assert_eq!(outcome.cost.by_provider[0].provider, "mock");
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
        // Model provenance (#159): only the local provider produced the gap-fills,
        // which is exactly what proves no frontier model touched this pipeline.
        assert_eq!(outcome.proposal.llm_providers, vec!["mock".to_string()]);
    }

    /// Air-gap zero-egress (#102): a full conversion with a frontier tripwire
    /// alongside the local provider, in air-gap mode, must complete **without ever
    /// calling the tripwire** — even when the routing policy lists the frontier
    /// provider first. If any non-local provider were reached, fill_gap would
    /// panic and fail this test.
    #[tokio::test]
    async fn air_gap_conversion_makes_zero_frontier_egress() {
        let importer = MockImporter;
        let local = MockLlmProvider; // is_local == true
        let frontier = EgressTripwire; // is_local == false; panics if called
                                       // Policy *prefers* the frontier provider — air-gap must still skip it.
        let policy = RoutingPolicy {
            bulk: vec!["frontier-tripwire".into(), "mock".into()],
            hard: vec!["frontier-tripwire".into(), "mock".into()],
            docs: vec!["frontier-tripwire".into(), "mock".into()],
        };
        let router = Router::new(vec![&frontier, &local], /* air_gap */ true).with_policy(policy);

        let outcome = convert_pipeline(
            &importer,
            &router,
            "SARC-main",
            "prop-eg",
            Classification::Yaml,
            "languages: dotnet",
        )
        .await
        .expect("air-gap conversion completes via the local provider only");

        // It converted, and provenance proves only the local model was used —
        // zero egress to the frontier tripwire.
        assert!(!outcome.proposal.proposed_yaml.is_empty());
        assert_eq!(outcome.proposal.llm_providers, vec!["mock".to_string()]);
        assert!(!outcome
            .proposal
            .llm_providers
            .iter()
            .any(|p| p == "frontier-tripwire"));
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
