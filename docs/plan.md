---
title: Implementation Plan
layout: default
nav_order: 7
permalink: /plan
---

# Implementation Plan
{: .no_toc }

> This page mirrors [`bifrost-implementation-plan.md`](https://github.com/olafkfreund/bifrost/blob/main/bifrost-implementation-plan.md)
> at the repo root, which remains the canonical source.

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

**Working codename:** Bifrost (the bridge between worlds).

**One-line thesis:** An orchestration + intelligence layer that sits on top of GitHub's existing
migration CLIs (`gh actions-importer`, GEI/`ado2gh`) and turns one-pipeline-at-a-time, syntactic,
CLI-only conversion into a portfolio-scale, semantically-reviewed, human-approved,
fully-documented migration from Azure DevOps to GitHub Actions — with a pluggable multi-model LLM
layer that can run fully air-gapped.

> We do **not** rebuild repo migration or YAML translation. GitHub already does ~90% of the
> syntactic conversion. Bifrost owns the **other 10%, the review/approval workflow, the portfolio
> orchestration, semantic-equivalence validation, and the audit trail** — the parts Microsoft is
> structurally disincentivised to build well.

## 1. Product principles (non-negotiables)

1. **Review-first, not autonomous.** The MVP recommends and explains; it never silently rewrites
   production CI. Auto-commit is opt-in and gated behind approval + validation.
2. **Air-gap capable.** Pipeline YAML leaks infra topology, secret *names*, and internal
   hostnames. A customer must be able to run Bifrost with a local model (Ollama / llama.cpp) so no
   pipeline definition ever leaves their network. This is a headline selling point.
3. **Platform-agnostic by design.** The Actions Importer already converts Jenkins, GitLab, Bamboo,
   Bitbucket, CircleCI, Travis. Bifrost's source adapter is an interface; ADO is just the first
   implementation.
4. **Attestation-native.** Every decision (who approved what, what changed, why, validation
   result) is recorded as a signed attestation. Compliance/audit export is a first-class feature.
5. **Wrap, don't fork.** We shell out to `gh actions-importer` and `gh`/`ado2gh`. We track their
   versions and parse their outputs. We never reimplement their conversion logic.

## 2. What we reuse vs what we build

| Capability | Owner | Bifrost's role |
|---|---|---|
| Repo migration (history, branches, metadata) | GEI / `ado2gh` | Orchestrate + track only |
| Near-zero-downtime repo sync | ELM (MS, preview) | Out of scope v1; integrate later |
| Pipeline syntactic conversion (~90%) | `gh actions-importer` | Wrap audit/forecast/dry-run/migrate |
| Audit footprint report | `gh actions-importer audit` | Parse + aggregate to portfolio view |
| **The unconverted ~10%** | **nobody** | **LLM-augmented gap fill + explanation** |
| **Semantic-equivalence validation** | **nobody** | **Trigger + diff against ADO baseline** |
| **Portfolio-scale orchestration** | **nobody** | **Job state machine + dashboard** |
| **Review / approve / edit UI** | **nobody** | **React portal w/ side-by-side diff** |
| **Migration documentation + attestation** | **nobody** | **Auto-generated runbook + signed record** |

## 3. Architecture (three planes)

```
┌─────────────────────────────────────────────────────────────────────┐
│  PORTAL (React/TS)                                                     │
│  portfolio heatmap · side-by-side diff · approve/edit · runbook view  │
└───────────────▲───────────────────────────────────────────────────────┘
                │ REST/SSE
┌───────────────┴───────────────────────────────────────────────────────┐
│  CONTROL PLANE (Rust / axum + tokio)                                   │
│  job state machine (PG) · conversion orchestrator · risk model ·       │
│  attestation + audit log                                               │
│                           │ LLM provider trait                         │
│        Anthropic (Claude) │ Gemini │ Copilot/llama.cpp │ Ollama (air-gap)│
└───────────────▲───────────────────────────────────────────────────────┘
                │ shell-out (Docker)        │ HTTP
┌───────────────┴───────────────┐ ┌─────────┴──────────────────────────┐
│  INGESTION ADAPTERS            │ │  EXTERNAL                          │
│  gh actions-importer (Docker)  │ │  ADO REST API · GitHub API · GEI   │
│  source adapter trait (ADO →…) │ │                                    │
└────────────────────────────────┘ └────────────────────────────────────┘
```

### Components

- **Source adapter trait** — `discover()`, `enumerate_pipelines()`, `fetch_definition()`,
  `fetch_service_connections()`, `fetch_variable_groups()`, `task_inventory()`. First impl:
  `AzureDevOpsAdapter` (ADO REST API). Distinguish **classic/designer** pipelines from **YAML**
  pipelines — classic ones are the painful tail.
- **Importer wrapper** — runs the official Docker image; parses `audit_summary.md` (Successful /
  Partially successful / Unsupported counts, Manual-tasks list, Unsupported-steps list, actions
  allowlist) and per-pipeline `dry-run` YAML + logs.
- **Conversion orchestrator** — for each pipeline: run dry-run → diff Importer output against
  source → identify gaps → dispatch gaps to LLM → assemble augmented workflow + rationale + risk.
- **Risk model** — deterministic scoring (see §6) → Green / Amber / Red.
- **Attestation + audit log** — append-only record of every state transition and human action.

## 4. Tech stack + rationale

- **Control plane: Rust (axum, tokio, sqlx).** Strong for a long-running orchestrator shelling
  out to containers and fanning out concurrent LLM calls. SSE for live job progress to the portal.
- **Persistence: Postgres** (multi-tenant/server), **SQLite** (single-tenant/local/air-gap).
  `sqlx` over both.
- **Ingestion: official `gh actions-importer` Docker image** + `ado2gh`/`gh` invoked as
  subprocesses. Pin and record tool versions per job.
- **LLM abstraction: `LlmProvider` trait** with impls for Anthropic, Google Gemini, GitHub
  Copilot/Models, and an OpenAI-compatible local impl (Ollama / llama.cpp). Structured JSON
  output, prompt templates versioned in-repo.
- **Portal: React + TypeScript + Vite + Tailwind**, Monaco editor for the three-pane diff (ADO
  YAML | generated Actions YAML | LLM rationale).
- **Auth: GitHub App** (installation tokens, least-privilege) + **Entra ID OIDC** for portal SSO.
- **Packaging: Docker Compose** (self-host v1) → **Helm chart** later.
- **Licence: MIT.** Consider open-core: OSS engine, commercial compliance/attestation +
  multi-tenant control plane.

## 5. Conversion + LLM pipeline (the core loop)

For each pipeline:

1. `dry-run` via Importer → baseline GHA YAML + conversion log.
2. Parse log for **unsupported steps**, **partial constructs**, **manual tasks** (secrets, service
   connections, self-hosted agents, environments/approval gates).
3. For each gap, build a **grounded** LLM request:
   - **Input:** source task/snippet (ADO) + Importer's converted YAML + the specific failure from
     the log + repo context (languages, detected build tools).
   - **Output (structured JSON):** `{ proposed_yaml, rationale, risk_flags[], verify_steps[],
     confidence }`.
   - The model fills **only the gap**, working from the Importer's diff — never converts from
     scratch. Keeps it grounded, cheap, and auditable.
4. **Model routing policy:** bulk/cheap classification + drafting → local model (Ollama) or
   Haiku-class; hard semantic reasoning + documentation → Claude/Gemini frontier. In air-gap mode,
   everything routes local and frontier providers are disabled by config.
5. Assemble augmented workflow; attach rationale + risk; persist as a **proposal** awaiting review.

## 6. Risk model (deterministic, explainable)

Score is computed from factors, not from the LLM (the LLM explains; it does not score):

- % of steps the Importer could not convert
- presence of secrets / variable groups (→ repo/org secrets to provision)
- service connections (→ OIDC federation to GitHub required)
- self-hosted agent pools (→ runner strategy decision)
- deployment/approval gates (→ Environments + required reviewers)
- matrix/parallelism semantics differences
- custom or marketplace tasks with no GHA equivalent
- artifact-passing semantics (publish/download → `actions/upload|download-artifact`)
- complex conditional expressions / template expansion

Weighted sum → **Green** (mechanical, low risk) / **Amber** (needs human verification) / **Red**
(manual rework / architectural decision required). Score + factor breakdown shown in the portal so
reviewers see *why*.

## 7. Validation / equivalence (the credibility feature)

Full proof of equivalence is impossible; we do **smoke parity**:

1. Push the converted workflow to a sandbox branch/repo.
2. Trigger via `workflow_dispatch`; capture status, jobs, produced artifacts, key outputs.
3. Compare against the last successful ADO run for the same pipeline (status, artifact set,
   declared outputs).
4. Emit a parity report + attestation. Reviewer sees pass/divergence before approving commit.

## 8. Build sequence (milestones)

- **M0 — Foundations.** Repo, monorepo layout, CI, licence, devcontainer, CLAUDE.md, docs site.
- **M1 — Audit MVP.** ADO adapter + Importer audit wrapper + portfolio heatmap (CLI + minimal
  read-only portal). *This alone demos the whole thesis.*
- **M2 — Conversion + LLM.** dry-run wrapping, gap detection, LLM provider trait + ≥2 impls
  (Anthropic + Ollama), grounded gap-fill, risk model.
- **M3 — Review portal.** Three-pane diff, approve/edit, proposal lifecycle.
- **M4 — Commit + PR.** `migrate`/push, manual-task checklists (secrets/runners/envs), PR open.
- **M5 — Validation.** Sandbox trigger + parity report.
- **M6 — Compliance + deploy.** Attestation export, GitHub App + Entra OIDC, multi-tenant, Helm.

**Recommended first cut to ship/demo:** M0 → M1 → the read-only recommendation slice of M2 (LLM
explanation + risk score, no auto-commit). Buildable in weeks on top of existing tools.

## 9. Driving this in Claude Code

1. Drop `CLAUDE.md` and this plan at the repo root.
2. Run `./seed-issues.sh` (needs `gh` authenticated) to create milestones, labels, epics, issues.
3. Work **one epic at a time, milestone order**. For each issue: let Claude Code read the issue +
   relevant plan section, implement, open a PR referencing the issue, and tick the epic checklist.
4. Keep `gh actions-importer` and `ado2gh` versions pinned in a `.tool-versions`-style file so
   conversions are reproducible and attestable.

## 10. Open decisions (resolve before/early in M2)

- **Open-core boundary:** what's MIT vs commercial? (Suggest: engine OSS; attestation +
  multi-tenant control plane commercial.)
- **Copilot as a provider:** via GitHub Models API, or treat "Copilot" as a positioning term and
  route to Claude/Gemini under the hood? (Confirm API access + ToS.)
- **Classic pipelines:** how far do we support designer/classic ADO pipelines vs YAML-only in v1?
  (Suggest YAML-first; classic = Amber/Red, flagged for manual.)
- **Hosted vs self-host first:** self-host/air-gap is the differentiator — lead with it.
- **Defensibility vs Microsoft:** assume MS bolts Copilot onto the Importer; double down on
  air-gap, multi-model, validation, attestation, and multi-platform sources.
