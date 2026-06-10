# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Bifrost is an orchestration + intelligence layer on top of GitHub's official migration CLIs
(`gh actions-importer`, GEI/`ado2gh`). It turns one-at-a-time, syntactic, CLI-only pipeline
conversion (Azure DevOps → GitHub Actions) into a portfolio-scale, semantically-reviewed,
human-approved, documented migration with a pluggable, air-gap-capable multi-model LLM layer.

We **wrap** the official tools; we never reimplement their conversion logic. Read
`bifrost-implementation-plan.md` for the full design — section numbers (§4, §6, …) referenced
below point into it.

## Current state — read this first

This repo is **pre-implementation**. As of now it contains only three files:

- `bifrost-implementation-plan.md` — the authoritative design (architecture, risk model, milestones).
- `seed-issues.sh` — bootstraps the GitHub backlog (milestones, labels, epics, issues).
- `CLAUDE.md` — this file.

There is **no Cargo workspace, no portal, and no code yet**, and the directory is not a git repo.
The "Target repo layout", "Stack", and "Commands" below describe what the milestones build toward,
not what exists. Do not assume a build exists — scaffolding it is the first M0 issue. The first
real work is `git init` + the M0 "Scaffold Cargo workspace" issue.

## How work is driven (issue-first, milestone-ordered)

1. `./seed-issues.sh` (needs `gh` authenticated; optionally `REPO=org/bifrost ./seed-issues.sh`)
   creates 7 milestones (M0–M6), the label taxonomy, epic tracking issues, and all child issues.
   **It is not idempotent for issues** — re-running duplicates them; run the issue section once
   (labels/milestones tolerate re-runs).
2. Work **one epic at a time, in milestone order**. For each issue: read the issue + the relevant
   plan section, implement, open **one PR per issue** with `Closes #<n>`, then tick the epic's
   checklist.
3. Milestone order is the build sequence (plan §8):
   - **M0 Foundations** — workspace, CI, devcontainer, MIT licence, docs site, Importer-output fixtures.
   - **M1 Audit MVP** — `SourceAdapter` trait + `AzureDevOpsAdapter` + Importer `audit` wrapper + portfolio heatmap. *This slice alone demos the thesis.*
   - **M2 Conversion + LLM** — `dry-run` wrapping, gap detection, `LlmProvider` trait + ≥2 impls (Anthropic + Ollama), deterministic risk model, axum API + persistence + job orchestration.
   - **M3 Review Portal** — three-pane diff, approve/edit, proposal lifecycle.
   - **M4 Commit + PR** — push/`migrate`, manual-task checklists, PR automation.
   - **M5 Validation** — sandbox `workflow_dispatch` trigger + parity report.
   - **M6 Compliance + Deploy** — attestation export, GitHub App + Entra OIDC, multi-tenancy, Helm.
   - Recommended first cut to demo: M0 → M1 → the read-only recommendation slice of M2.
4. Issue labels encode where work lands: `plane:{ingestion,control,portal,llm}`,
   `area:{validation,compliance}`, `type:{feature,chore,spike,docs}`, `priority:{p0,p1,p2}`, `epic`.

## Hard rules (non-negotiable — plan §1)

- **Review-first.** Never silently rewrite production CI. Auto-commit is opt-in, gated behind
  human approval + validation. Default path = recommend + explain.
- **Air-gap capable.** A customer must be able to run with a local model only (Ollama/llama.cpp);
  in air-gap mode, frontier providers are disabled by config and no pipeline data leaves the box.
  There is an explicit test target asserting *zero external calls* in air-gap mode.
- **The LLM explains; it does not score.** Risk scoring is deterministic (plan §6). The LLM
  fills gaps, explains, and flags — it never produces the numeric risk score.
- **Grounded generation only.** LLM requests carry the source snippet + the Importer's converted
  output + the specific failure from the log + repo context. The model fills the gap from that
  diff — it does not convert pipelines from scratch.
- **Everything is attestable.** Every state transition and human action is appended to an
  immutable audit log. Treat this as a feature, not logging.
- **Wrap, don't fork.** Shell out to `gh actions-importer` / `gh` / `ado2gh`; parse their output.
  **Pin and record tool versions + image digest for every job** so conversions are reproducible
  and attestable.

## Architecture (three planes — plan §3)

```
PORTAL (React/TS)           portfolio heatmap · 3-pane diff · approve/edit · runbook
      │ REST/SSE
CONTROL PLANE (Rust/axum)   job state machine (PG/SQLite) · conversion orchestrator ·
      │                     deterministic risk model · attestation + audit log
      │ LlmProvider trait → Anthropic · Gemini · Copilot/Models · Ollama (air-gap)
      ▼ shell-out (Docker)            ▼ HTTP
INGESTION ADAPTERS          EXTERNAL: ADO REST API · GitHub API · GEI
  gh actions-importer (Docker)
  SourceAdapter trait (ADO → …)
```

The core conversion loop (plan §5), per pipeline: Importer `dry-run` → parse log for unsupported
steps / partial constructs / manual tasks into typed **Gap** records → build a grounded LLM
request per gap → assemble augmented workflow + rationale + risk → persist as a **Proposal**
awaiting review. Proposal lifecycle:
`draft → in_review → approved/changes_requested → committed → validated`; illegal transitions are
rejected and every transition is logged to the audit log.

Two key seams that must stay abstract:
- **`SourceAdapter` trait** — `discover` / `enumerate_pipelines` / `fetch_definition` /
  `fetch_service_connections` / `fetch_variable_groups` / `task_inventory`. ADO is just the first
  impl; keep platform-agnostic (the Importer also handles Jenkins/GitLab/etc.). Classic/designer
  pipelines are the hard tail — classify them separately from YAML pipelines.
- **`LlmProvider` trait** — returns structured JSON
  `{ proposed_yaml, rationale, risk_flags[], verify_steps[], confidence }`. Orchestration calls
  **only** this trait, never a vendor SDK directly. Routing policy: bulk/cheap → local/Haiku-class;
  hard reasoning/docs → frontier; air-gap mode forces everything local.

## Target repo layout

```
/crates
  /bifrost-core        # domain types, job state machine, risk model
  /bifrost-adapters    # source adapter trait + AzureDevOpsAdapter; Importer wrapper
  /bifrost-llm         # LlmProvider trait + Anthropic/Gemini/Copilot/Ollama impls
  /bifrost-api         # axum control-plane API + SSE
  /bifrost-cli         # CLI entrypoint (audit, convert, report)
/portal                # React + TS + Vite + Tailwind; Monaco three-pane diff
/prompts               # versioned LLM prompt templates (referenced by id, auditable)
/fixtures              # captured audit_summary.md / dry-run YAML+logs for tests
/deploy                # docker-compose, later Helm
/docs                  # Jekyll docs site
```

## Stack (target — plan §4)

Rust (axum, tokio, sqlx) · **Postgres** (server/multi-tenant) / **SQLite** (local/air-gap), same
schema over both · official Importer Docker image shelled out as a subprocess ·
React/TS/Vite/Tailwind/Monaco · GitHub App + Entra ID OIDC · Docker Compose → Helm · MIT licence.

## Commands (target — none work until the M0 scaffold lands)

Once the Cargo workspace exists, the intended dev commands (from the M0 issues) are:

```bash
cargo build                          # build the workspace
cargo fmt --check                    # formatting gate
cargo clippy -- -D warnings          # lint gate (clippy is a required CI check)
cargo test                           # run all tests
cargo test -p bifrost-core <name>    # single crate / single test by name
```

CI runs fmt check + `clippy -D warnings` + build + test on push/PR as a required check. Tests for
the risk model and log parsers run against captured Importer fixtures in `/fixtures`.

## Conventions

- **Conventional Commits.** One PR per issue; PR body must include `Closes #<n>`.
- Every external tool invocation (Importer, `gh`, `ado2gh`, ADO/GitHub REST) is wrapped behind a
  **trait** so it can be mocked in tests.
- LLM calls go through the `LlmProvider` trait — never a vendor SDK from orchestration code.
- **No secrets in code or logs.** Secret *names* discovered during audit are data; secret *values*
  are never fetched or stored. Variable groups / service connections record names + types only.
- Tests: unit-test the risk model and log parsers against captured Importer fixtures; record real
  `audit_summary.md` / dry-run outputs as fixtures so behaviour is reproducible.

## Definition of done (per issue)

Code + tests + docs updated, PR references the issue (`Closes #<n>`), CI green, and — where
relevant — a captured fixture added so behaviour is reproducible.

## Domain glossary

- **Importer** = `gh actions-importer` (audit/forecast/dry-run/migrate).
- **GEI / ado2gh** = repo migration tooling (history/branches/metadata). Out of scope for
  conversion; orchestrate/track only.
- **Gap** = a construct the Importer marked partial/unsupported, routed to the LLM.
- **Proposal** = an augmented workflow + rationale + risk, awaiting human review.
- **Parity report** = smoke-validation result comparing a converted workflow run to the ADO
  baseline (status, artifact set, declared outputs — smoke parity, *not* full equivalence).
- **Classic pipeline** = ADO designer pipeline (the hard tail, defaults Amber/Red);
  **YAML pipeline** = the easy path.
