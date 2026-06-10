#!/usr/bin/env bash
#
# seed-issues.sh — bootstrap the Bifrost backlog in GitHub.
#
# Prereqs:  gh auth login   (and `gh repo set-default` OR run inside the repo)
# Usage:    ./seed-issues.sh            # creates everything in the default repo
#           REPO=org/bifrost ./seed-issues.sh
#
# Idempotency: label/milestone creation tolerates "already exists". Re-running will
# create DUPLICATE issues, so run the issue section once. Comment out blocks to re-run safely.
#
set -euo pipefail

REPO_FLAG=()
if [[ -n "${REPO:-}" ]]; then REPO_FLAG=(--repo "$REPO"); fi

say() { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }

# ---------------------------------------------------------------------------
# Labels
# ---------------------------------------------------------------------------
say "Creating labels"
ensure_label() { gh label create "$1" --color "$2" --description "$3" "${REPO_FLAG[@]}" 2>/dev/null || \
                 gh label edit   "$1" --color "$2" --description "$3" "${REPO_FLAG[@]}" 2>/dev/null || true; }

ensure_label "epic"            "6f42c1" "Epic tracking issue"
ensure_label "type:feature"    "0e8a16" "New capability"
ensure_label "type:chore"      "fbca04" "Infra / tooling / housekeeping"
ensure_label "type:spike"      "d4c5f9" "Time-boxed investigation"
ensure_label "type:docs"       "0075ca" "Documentation"
ensure_label "plane:ingestion" "1d76db" "Ingestion / source adapters"
ensure_label "plane:control"   "5319e7" "Control plane / orchestration"
ensure_label "plane:portal"    "e99695" "Review portal"
ensure_label "plane:llm"       "c2e0c6" "LLM provider layer"
ensure_label "area:validation" "bfd4f2" "Equivalence / parity validation"
ensure_label "area:compliance" "f9d0c4" "Attestation / audit / governance"
ensure_label "priority:p0"     "b60205" "Must-have for the milestone"
ensure_label "priority:p1"     "d93f0b" "Important"
ensure_label "priority:p2"     "fef2c0" "Nice-to-have"
ensure_label "good-first-issue" "7057ff" "Good entry point"

# ---------------------------------------------------------------------------
# Milestones
# ---------------------------------------------------------------------------
say "Creating milestones"
ensure_milestone() {
  gh api "repos/{owner}/{repo}/milestones" "${REPO_FLAG[@]}" -f title="$1" -f description="$2" >/dev/null 2>&1 \
    || true
}
ensure_milestone "M0 - Foundations"        "Repo, CI, devcontainer, docs, fixtures"
ensure_milestone "M1 - Audit MVP"          "ADO adapter + Importer audit + portfolio heatmap"
ensure_milestone "M2 - Conversion + LLM"   "dry-run wrapping, gap detection, LLM layer, risk model"
ensure_milestone "M3 - Review Portal"      "Three-pane diff, approve/edit, proposal lifecycle"
ensure_milestone "M4 - Commit + PR"        "Push/migrate, manual-task checklists, PR automation"
ensure_milestone "M5 - Validation"         "Sandbox trigger + parity report"
ensure_milestone "M6 - Compliance + Deploy" "Attestation export, auth, multi-tenant, Helm"

# ---------------------------------------------------------------------------
# Issue helper
#   mk <milestone> <comma-labels> <title> <body>
# ---------------------------------------------------------------------------
mk() {
  local ms="$1" labels="$2" title="$3" body="$4"
  # gh expects comma-separated labels in a single --label flag; the table above
  # uses ';' as the separator, so translate before handing it to gh.
  gh issue create "${REPO_FLAG[@]}" \
    --title "$title" \
    --body  "$body" \
    --milestone "$ms" \
    --label "${labels//;/,}" >/dev/null
  printf '  + %s\n' "$title"
}

# ===========================================================================
# EPIC TRACKING ISSUES
# ===========================================================================
say "Creating epic tracking issues"
mk "M0 - Foundations" "epic" \
  "EPIC: Foundations & project scaffolding" \
  $'Stand up the monorepo, CI, dev environment, licence, docs site, and test-fixture harness so every later epic builds on a stable base.\n\nChild issues carry the M0 milestone. Wire as sub-issues once created.'
mk "M1 - Audit MVP" "epic;plane:ingestion" \
  "EPIC: ADO discovery & ingestion" \
  $'Source-adapter trait + Azure DevOps implementation: enumerate the org, classify pipelines, and inventory the constructs that drive migration risk.\n\nGoal: feed the audit/portfolio view. No conversion yet.'
mk "M1 - Audit MVP" "epic;plane:ingestion" \
  "EPIC: GitHub Actions Importer integration" \
  $'Wrap the official Importer Docker image (audit/forecast/dry-run/migrate) and parse its outputs into typed domain data. We wrap; we never reimplement.'
mk "M2 - Conversion + LLM" "epic;plane:control" \
  "EPIC: Conversion engine & risk model" \
  $'Gap detection from Importer output, deterministic risk scoring, proposal lifecycle, and portfolio aggregation.'
mk "M2 - Conversion + LLM" "epic;plane:llm" \
  "EPIC: LLM provider layer & routing" \
  $'Pluggable multi-model layer (Anthropic/Gemini/Copilot/Ollama) with grounded gap-fill, structured output, routing policy, and air-gap mode.'
mk "M2 - Conversion + LLM" "epic;plane:control" \
  "EPIC: Control-plane API & state" \
  $'axum API, persistence (Postgres + SQLite), job state machine, live progress (SSE), and the append-only audit log.'
mk "M3 - Review Portal" "epic;plane:portal" \
  "EPIC: Review portal" \
  $'React portal: portfolio heatmap, three-pane diff (ADO | generated | rationale), approve/edit, and runbook view.'
mk "M4 - Commit + PR" "epic;plane:control" \
  "EPIC: Commit & PR automation" \
  $'Push approved workflows / open PRs via migrate, plus generated manual-task checklists (secrets, runners, environments, service connections).'
mk "M5 - Validation" "epic;area:validation" \
  "EPIC: Validation & equivalence" \
  $'Smoke-parity: trigger the converted workflow in a sandbox, capture the run, diff against the ADO baseline, emit a parity report.'
mk "M6 - Compliance + Deploy" "epic;area:compliance" \
  "EPIC: Compliance, auth & deployment" \
  $'Signed attestation export, GitHub App + Entra OIDC auth, multi-tenancy, and packaging (Compose → Helm).'

# ===========================================================================
# M0 — Foundations
# ===========================================================================
say "M0 issues"
mk "M0 - Foundations" "type:chore;priority:p0;good-first-issue" \
  "Scaffold Cargo workspace + crate layout" \
  $'Create the workspace with crates: bifrost-core, bifrost-adapters, bifrost-llm, bifrost-api, bifrost-cli. Add rustfmt + clippy config.\n\nAC: `cargo build` and `cargo clippy` pass on an empty skeleton.'
mk "M0 - Foundations" "type:chore;priority:p0" \
  "CI pipeline (build, fmt, clippy, test)" \
  $'GitHub Actions workflow running fmt check, clippy -D warnings, build, and test on push/PR.\n\nAC: green required-check on PRs.'
mk "M0 - Foundations" "type:chore;priority:p1" \
  "Devcontainer + toolchain pinning" \
  $'Devcontainer with Rust, Docker-in-Docker (for the Importer image), Node (portal), and `gh`. Add a `.tool-versions`-style file pinning gh / actions-importer / ado2gh versions.\n\nAC: clone-to-build works in a fresh container.'
mk "M0 - Foundations" "type:docs;priority:p1;good-first-issue" \
  "Licence (MIT), README, CONTRIBUTING" \
  $'MIT licence, README stating the wrap-don\x27t-fork thesis, CONTRIBUTING with Conventional Commits + one-PR-per-issue rule.'
mk "M0 - Foundations" "type:docs;priority:p2" \
  "Docs site (Jekyll) skeleton" \
  $'Jekyll docs site with sections matching the implementation plan. Publish via Pages.\n\nAC: site builds in CI.'
mk "M0 - Foundations" "type:chore;priority:p0" \
  "Importer-output fixture harness" \
  $'Capture real `audit_summary.md` and `dry-run` YAML/logs from a sample ADO org into /fixtures. Provide a loader for tests.\n\nAC: fixtures load in unit tests; used by the parser issues.'
mk "M0 - Foundations" "type:spike;priority:p1" \
  "SPIKE: confirm Copilot/GitHub Models API access + ToS" \
  $'Determine whether \x27Copilot\x27 is a usable provider via GitHub Models API, or a positioning term routed to Claude/Gemini. Document constraints for the LLM epic.'

# ===========================================================================
# M1 — ADO ingestion
# ===========================================================================
say "M1 ingestion issues"
mk "M1 - Audit MVP" "plane:ingestion;type:feature;priority:p0" \
  "Define SourceAdapter trait" \
  $'Trait: discover, enumerate_pipelines, fetch_definition, fetch_service_connections, fetch_variable_groups, task_inventory. ADO is the first impl; keep platform-agnostic.\n\nAC: trait + mock impl + doc comments.'
mk "M1 - Audit MVP" "plane:ingestion;type:feature;priority:p0" \
  "ADO auth (PAT + Entra) and client" \
  $'Authenticated ADO REST client supporting PAT and Entra tokens, with retry/backoff and rate-limit handling.\n\nAC: integration test against a test org (or recorded cassettes).'
mk "M1 - Audit MVP" "plane:ingestion;type:feature;priority:p0" \
  "Enumerate projects & pipelines" \
  $'List all projects and their build/release definitions across the org.\n\nAC: returns typed Pipeline records with ids, names, project, repo link.'
mk "M1 - Audit MVP" "plane:ingestion;type:feature;priority:p0" \
  "Classify classic vs YAML pipelines" \
  $'Tag each pipeline as classic/designer or YAML. Classic = the hard tail (default Amber/Red downstream).\n\nAC: classification field populated; counts surfaced.'
mk "M1 - Audit MVP" "plane:ingestion;type:feature;priority:p1" \
  "Fetch service connections" \
  $'Enumerate service connections (and types) per project — drives the OIDC-federation risk factor. Record names/types only, never secrets.'
mk "M1 - Audit MVP" "plane:ingestion;type:feature;priority:p1" \
  "Fetch variable groups (names only)" \
  $'Enumerate variable groups and variable *names* (mark secret-flagged ones). Never fetch secret values.'
mk "M1 - Audit MVP" "plane:ingestion;type:feature;priority:p1" \
  "Task/extension inventory" \
  $'Aggregate which built-in/marketplace/custom tasks are used across the org and how often — feeds the unsupported-task risk factor and an org-wide allowlist view.'

# ===========================================================================
# M1 — Importer integration
# ===========================================================================
say "M1 importer issues"
mk "M1 - Audit MVP" "plane:ingestion;type:feature;priority:p0" \
  "Importer Docker wrapper" \
  $'Run the official `gh actions-importer` image as a subprocess with configured creds via env/.env.local. Capture stdout/stderr and exit codes. Record the image digest used.\n\nAC: wrapper invokes `version` and `audit` against a fixture/test org.'
mk "M1 - Audit MVP" "plane:ingestion;type:feature;priority:p0" \
  "Parse audit_summary.md" \
  $'Parse Successful / Partially successful / Unsupported counts, the Manual-tasks list, Unsupported-steps list, and actions allowlist into typed structs.\n\nAC: unit tests against captured fixtures.'
mk "M1 - Audit MVP" "plane:ingestion;type:feature;priority:p1" \
  "Wrap forecast command" \
  $'Run `forecast` and parse projected Actions usage / runner-minutes. Surface as cost input for the portfolio view.'
mk "M2 - Conversion + LLM" "plane:ingestion;type:feature;priority:p0" \
  "Wrap dry-run + parse conversion log" \
  $'Run `dry-run` per pipeline; capture converted YAML and parse the log for unsupported steps / partial constructs / manual tasks into structured Gap records.\n\nAC: Gap list produced from fixtures.'
mk "M1 - Audit MVP" "plane:ingestion;type:chore;priority:p1" \
  "Pin & record Importer/ado2gh versions per job" \
  $'Persist the exact tool versions + image digest used for each audit/convert run so results are reproducible and attestable.'

# ===========================================================================
# M2 — Conversion engine & risk
# ===========================================================================
say "M2 conversion/risk issues"
mk "M2 - Conversion + LLM" "plane:control;type:feature;priority:p0" \
  "Gap detector" \
  $'From dry-run output, produce a typed list of Gaps (unsupported step, partial construct, manual task) with the source snippet + Importer failure attached. Input for the LLM layer.'
mk "M2 - Conversion + LLM" "plane:control;type:feature;priority:p0" \
  "Deterministic risk model + scoring" \
  $'Implement the weighted factor model (plan §6) → Green/Amber/Red with a factor breakdown. The LLM must NOT influence the score.\n\nAC: unit-tested against fixtures; breakdown serialisable for the portal.'
mk "M2 - Conversion + LLM" "plane:control;type:feature;priority:p0" \
  "Proposal model + state machine" \
  $'Define Proposal (augmented workflow + rationale + risk + status) and its lifecycle: draft → in_review → approved/changes_requested → committed → validated.\n\nAC: illegal transitions rejected; transitions logged to the audit log.'
mk "M2 - Conversion + LLM" "plane:control;type:feature;priority:p1" \
  "Workflow assembler" \
  $'Merge Importer baseline YAML with LLM gap-fills into a single proposed workflow, preserving comments/anchors and recording provenance per block (importer vs llm).'
mk "M2 - Conversion + LLM" "plane:control;type:feature;priority:p1" \
  "Portfolio aggregation" \
  $'Roll per-pipeline classification + risk + forecast into an org-level summary (heatmap data) the API/portal consume.'

# ===========================================================================
# M2 — LLM layer
# ===========================================================================
say "M2 LLM issues"
mk "M2 - Conversion + LLM" "plane:llm;type:feature;priority:p0" \
  "Define LlmProvider trait + structured output" \
  $'Trait returning structured JSON {proposed_yaml, rationale, risk_flags[], verify_steps[], confidence}. Orchestration calls only the trait, never a vendor SDK directly.'
mk "M2 - Conversion + LLM" "plane:llm;type:feature;priority:p0" \
  "Grounded gap-fill request builder" \
  $'Build requests carrying source snippet + Importer-converted YAML + specific log failure + repo context. The model fills the gap from the diff; it never converts from scratch.'
mk "M2 - Conversion + LLM" "plane:llm;type:feature;priority:p0" \
  "Anthropic (Claude) provider impl" \
  $'Implement LlmProvider for Anthropic. Use for hard semantic reasoning + documentation. Config-driven model selection.'
mk "M2 - Conversion + LLM" "plane:llm;type:feature;priority:p0" \
  "Ollama / llama.cpp (local) provider impl" \
  $'OpenAI-compatible local provider for air-gap/bulk work. Must be sufficient to run the whole pipeline with zero external calls.'
mk "M2 - Conversion + LLM" "plane:llm;type:feature;priority:p1" \
  "Gemini provider impl" \
  $'Implement LlmProvider for Google Gemini.'
mk "M2 - Conversion + LLM" "plane:llm;type:feature;priority:p2" \
  "Copilot / GitHub Models provider impl" \
  $'Implement provider per the outcome of the M0 spike (or document why it routes elsewhere).'
mk "M2 - Conversion + LLM" "plane:llm;type:feature;priority:p0" \
  "Routing policy + air-gap mode" \
  $'Config-driven routing: bulk/cheap → local/Haiku-class; hard reasoning/docs → frontier. Air-gap mode disables all non-local providers and asserts no egress.\n\nAC: air-gap mode test proves no external calls.'
mk "M2 - Conversion + LLM" "plane:llm;type:chore;priority:p1" \
  "Versioned prompt templates" \
  $'Store prompt templates in /prompts with versioning; templates referenced by id so changes are auditable.'

# ===========================================================================
# M2/M3 — Control plane API & state
# ===========================================================================
say "M2/M3 control-plane issues"
mk "M2 - Conversion + LLM" "plane:control;type:feature;priority:p0" \
  "axum API skeleton + SSE" \
  $'API for orgs/audits/pipelines/proposals + an SSE stream for live job progress to the portal.'
mk "M2 - Conversion + LLM" "plane:control;type:feature;priority:p0" \
  "Persistence: Postgres schema + migrations" \
  $'sqlx migrations for orgs, pipelines, gaps, proposals, jobs, audit_log. Server/multi-tenant mode.'
mk "M2 - Conversion + LLM" "plane:control;type:feature;priority:p1" \
  "SQLite (local/air-gap) mode" \
  $'Same schema over SQLite for single-tenant/local/air-gap deployments. Selectable by config.'
mk "M2 - Conversion + LLM" "plane:control;type:feature;priority:p0" \
  "Job orchestration (audit → convert fan-out)" \
  $'Async job runner that audits an org then fans out per-pipeline conversion + LLM gap-fill with bounded concurrency and resumability.'
mk "M2 - Conversion + LLM" "area:compliance;type:feature;priority:p0" \
  "Append-only audit log" \
  $'Immutable record of every state transition + human action (actor, before/after, timestamp). Foundation for attestation export.'

# ===========================================================================
# M3 — Review portal
# ===========================================================================
say "M3 portal issues"
mk "M3 - Review Portal" "plane:portal;type:chore;priority:p0" \
  "Portal scaffold (Vite + TS + Tailwind)" \
  $'React/TS/Vite/Tailwind app with auth-aware shell and API client + SSE subscription.'
mk "M3 - Review Portal" "plane:portal;type:feature;priority:p0" \
  "Portfolio heatmap" \
  $'Org-level dashboard: pipelines by classification + risk (Green/Amber/Red), forecast, progress. Drill into a pipeline.'
mk "M3 - Review Portal" "plane:portal;type:feature;priority:p0" \
  "Three-pane diff (ADO | generated | rationale)" \
  $'Monaco-based view: source ADO YAML, generated Actions YAML (provenance-highlighted), and LLM rationale + risk-factor breakdown + verify steps.'
mk "M3 - Review Portal" "plane:portal;type:feature;priority:p0" \
  "Approve / request-changes / edit" \
  $'Reviewer can edit the generated YAML, approve, or request changes; actions hit the proposal state machine and audit log.'
mk "M3 - Review Portal" "plane:portal;type:feature;priority:p1" \
  "Proposal lifecycle UI + queues" \
  $'Review queue filtered by status/risk; show who approved what and when.'
mk "M3 - Review Portal" "plane:portal;type:feature;priority:p2" \
  "Runbook view" \
  $'Render the generated per-pipeline migration runbook (manual tasks + verify steps) in-portal.'

# ===========================================================================
# M4 — Commit + PR
# ===========================================================================
say "M4 commit/PR issues"
mk "M4 - Commit + PR" "plane:control;type:feature;priority:p0" \
  "Manual-task checklist generator" \
  $'From gaps + risk, generate the checklist of things the Importer can\x27t do: secrets to provision, service connections → OIDC, self-hosted runners, environments/approval gates.'
mk "M4 - Commit + PR" "plane:control;type:feature;priority:p0" \
  "Commit approved workflow / open PR" \
  $'Push the approved workflow to a branch and open a PR (via `migrate` or direct push + gh), body linking the proposal + checklist. Opt-in; never silent.'
mk "M4 - Commit + PR" "plane:control;type:feature;priority:p1" \
  "Manual-task tracker" \
  $'Track checklist completion per pipeline; block \x27done\x27 until required items are resolved.'

# ===========================================================================
# M5 — Validation
# ===========================================================================
say "M5 validation issues"
mk "M5 - Validation" "area:validation;type:feature;priority:p0" \
  "Sandbox trigger (workflow_dispatch)" \
  $'Push converted workflow to a sandbox branch/repo and trigger it; isolate from production.'
mk "M5 - Validation" "area:validation;type:feature;priority:p0" \
  "Capture run result + artifacts" \
  $'Collect status, jobs, produced artifacts, and declared outputs from the converted run.'
mk "M5 - Validation" "area:validation;type:feature;priority:p0" \
  "Parity diff vs ADO baseline" \
  $'Compare converted run against the last successful ADO run (status, artifact set, outputs). Smoke parity, not full equivalence — be explicit about limits.'
mk "M5 - Validation" "area:validation;area:compliance;type:feature;priority:p1" \
  "Parity report + attestation" \
  $'Emit a parity report and record it as an attestation on the proposal; surface pass/divergence in the portal before commit approval.'

# ===========================================================================
# M6 — Compliance + deploy
# ===========================================================================
say "M6 compliance/deploy issues"
mk "M6 - Compliance + Deploy" "area:compliance;type:feature;priority:p0" \
  "Signed attestation record + export" \
  $'Produce signed, exportable attestations per migration (decisions, approvals, validation). Consider Kosli-style provenance / in-toto format.'
mk "M6 - Compliance + Deploy" "area:compliance;type:feature;priority:p1" \
  "Compliance export (audit pack)" \
  $'Export a per-org audit pack (who/what/why/when + parity results) as a single artifact for auditors.'
mk "M6 - Compliance + Deploy" "plane:control;type:feature;priority:p0" \
  "GitHub App auth (least privilege)" \
  $'GitHub App with installation tokens scoped to the minimum required for audit/convert/PR.'
mk "M6 - Compliance + Deploy" "plane:control;type:feature;priority:p1" \
  "Entra ID OIDC SSO for portal" \
  $'Portal SSO via Entra ID OIDC; map identities to reviewer roles.'
mk "M6 - Compliance + Deploy" "plane:control;type:feature;priority:p1" \
  "Multi-tenancy + RBAC" \
  $'Tenant isolation in data model + API; roles (admin/reviewer/viewer).'
mk "M6 - Compliance + Deploy" "type:chore;priority:p1" \
  "Packaging: docker-compose then Helm" \
  $'docker-compose for self-host v1; Helm chart for EKS/AKS/GKE (reuse SARC patterns).'

say "Done. Review issues, then wire epics → children as GitHub sub-issues if desired."
