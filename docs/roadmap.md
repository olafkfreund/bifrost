---
title: Roadmap
layout: default
nav_order: 3
permalink: /roadmap
---

# Roadmap
{: .no_toc }

Bifrost ships in seven milestones. Work is **issue-driven** and **milestone-ordered** — one epic
at a time, one PR per issue. The backlog is bootstrapped by
[`seed-issues.sh`](https://github.com/olafkfreund/bifrost/blob/main/seed-issues.sh).

1. TOC
{:toc}

---

## M0 — Foundations
{: .label .label-yellow }Current

Stand up the monorepo, CI, dev environment, licence, docs site, and test-fixture harness so every
later epic builds on a stable base.

- Scaffold Cargo workspace (`bifrost-core`, `-adapters`, `-llm`, `-api`, `-cli`)
- CI: build, fmt, `clippy -D warnings`, test
- Devcontainer + toolchain pinning (Rust, Docker-in-Docker, Node, `gh`)
- MIT licence, README, CONTRIBUTING
- Jekyll docs site (this site)
- Importer-output fixture harness (`audit_summary.md`, dry-run YAML/logs)

## M1 — Audit MVP

ADO discovery + Importer audit wrapper + a portfolio heatmap. **This slice alone demos the whole
thesis.**

- `SourceAdapter` trait + `AzureDevOpsAdapter` (PAT + Entra auth)
- Enumerate projects & pipelines; classify classic vs YAML
- Fetch service connections + variable groups (**names only, never secret values**)
- Task/extension inventory across the org
- Importer Docker wrapper + parse `audit_summary.md`
- Forecast wrapping; pin & record tool versions per job

## M2 — Conversion + LLM

dry-run wrapping, gap detection, the pluggable LLM layer, and the deterministic risk model.

- Gap detector + deterministic risk model (Green/Amber/Red)
- Proposal model + state machine (`draft → in_review → approved/changes_requested → committed → validated`)
- `LlmProvider` trait + structured output; grounded gap-fill request builder
- Anthropic (Claude) + Ollama/llama.cpp providers; Gemini; routing policy + **air-gap mode**
- axum API + SSE; Postgres + SQLite persistence; job orchestration; append-only audit log

## M3 — Review Portal

The React portal where humans review and approve.

- Vite + TS + Tailwind scaffold; portfolio heatmap
- Three-pane diff (ADO YAML \| generated Actions YAML \| LLM rationale, Monaco)
- Approve / request-changes / edit; proposal lifecycle UI; runbook view

## M4 — Commit + PR

- Manual-task checklist generator (secrets, service connections → OIDC, runners, environments)
- Commit approved workflow / open PR (opt-in, never silent)
- Manual-task tracker

## M5 — Validation

- Sandbox trigger via `workflow_dispatch`; capture run result + artifacts
- Parity diff vs ADO baseline (smoke parity, **not** full equivalence)
- Parity report + attestation surfaced before commit approval

## M6 — Compliance + Deploy

- Signed attestation record + export (consider in-toto / provenance format)
- Compliance audit pack export
- GitHub App auth (least privilege) + Entra ID OIDC SSO
- Multi-tenancy + RBAC; packaging (Docker Compose → Helm)

---

> **Recommended first cut to demo:** M0 → M1 → the read-only recommendation slice of M2 (LLM
> explanation + risk score, no auto-commit).
