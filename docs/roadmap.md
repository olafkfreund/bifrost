---
title: Roadmap
layout: default
nav_order: 4
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
{: .label .label-yellow }In progress

Stand up the monorepo, CI, dev environment, licence, docs site, and test-fixture harness so every
later epic builds on a stable base.

- Scaffold Cargo workspace (`bifrost-core`, `-adapters`, `-llm`, `-api`, `-cli`) — done
- CI: build, fmt, `clippy -D warnings`, test — done
- Jekyll docs site (this site) — done
- Devcontainer + toolchain pinning (Rust, Docker-in-Docker, Node, `gh`) — planned
- MIT licence, README, CONTRIBUTING — README done; licence + CONTRIBUTING planned
- Importer-output fixture harness (`audit_summary.md`, dry-run YAML/logs) — planned

## M1 — Audit MVP
{: .label .label-green }Mostly done

ADO discovery + Importer audit wrapper + a portfolio heatmap. **This slice alone demos the whole
thesis** — and it runs against live Azure DevOps projects today.

- `SourceAdapter` trait + `AzureDevOpsAdapter` — done
- Enumerate projects & pipelines; classify classic vs YAML — done
- Fetch service connections + variable groups (**names only, never secret values**) — done
- Task/extension inventory across the org — done
- Importer Docker wrapper + parse `audit_summary.md` — done
- ADO auth (PAT done; Entra ID planned), forecast wrapping, per-job version pinning — planned

## M2 — Conversion + LLM
{: .label .label-green }Done

dry-run wrapping, gap detection, the pluggable LLM layer, and the deterministic risk model.

- Gap detector + deterministic risk model (Green/Amber/Red) — done
- Proposal model + state machine (`draft → in_review → approved/changes_requested → committed → validated`) — done
- `LlmProvider` trait + structured output; grounded gap-fill request builder — done
- Anthropic (Claude), Gemini, Copilot/Models, and Ollama/llama.cpp providers; routing policy + **air-gap mode** — done
- axum API + SSE; Postgres + SQLite persistence; job orchestration; append-only audit log — done

## M3 — Review Portal
{: .label .label-green }Done

The React portal where humans review and approve.

- Vite + TS + Tailwind scaffold; portfolio heatmap — done
- Three-pane diff (ADO YAML \| generated Actions YAML \| LLM rationale, Monaco) — done
- Approve / request-changes / edit; proposal lifecycle UI; runbook view; in-portal docs — done

## M4 — Commit + PR
{: .label .label-green }Done

- Manual-task checklist generator (secrets, service connections → OIDC, runners, environments) — done
- Commit approved workflow / open PR (opt-in, never silent) — done
- Manual-task tracker gating the terminal state — done

## M5 — Validation
{: .label .label-yellow }In progress

- Sandbox trigger via `workflow_dispatch` — done
- Capture run result + artifacts (status, jobs, declared outputs) — done
- Parity diff vs ADO baseline (smoke parity, **not** full equivalence) — done
- Parity report + attestation surfaced before commit approval — planned

## M6 — Compliance + Deploy
{: .label .label-blue }Planned

- Signed attestation record + export (consider in-toto / provenance format)
- Compliance audit pack export
- GitHub App auth (least privilege) + Entra ID OIDC SSO
- Multi-tenancy + RBAC; packaging (Docker Compose → Helm)

---

> **Where we are:** the conversion loop is complete and runs end-to-end against live Azure DevOps
> projects (audit → convert → review → approve → PR → sandbox-validate). Current focus is
> finishing **M5 Validation** (parity report + attestation), then **M6 Compliance + Deploy**.
