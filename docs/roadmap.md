---
title: Roadmap
layout: default
nav_order: 7
permalink: /roadmap
---

# Roadmap
{: .no_toc }

Bifrost ships in eight milestones. Work is **issue-driven** and **milestone-ordered** — one epic
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
- Enterprise cloud LLM providers: **Azure OpenAI**, **GCP Vertex AI** (Gemini), **AWS Bedrock** (via an OpenAI-compatible gateway); private endpoints are air-gap-eligible — done
- Runtime **air-gap toggle** (settings, not just env); air-gap = in-network providers only — done
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
{: .label .label-green }Done

- Sandbox trigger via `workflow_dispatch` — done
- Capture run result + artifacts (status, jobs, declared outputs) — done
- Parity diff vs ADO baseline (smoke parity, **not** full equivalence) — done
- Parity report + attestation recorded on the proposal before validation — done

## M6 — Compliance + Deploy
{: .label .label-green }Done

- Signed, exportable per-migration attestation (in-toto-inspired, HMAC-signed) — done
- Per-org compliance audit pack export — done
- GitHub App auth (least-privilege installation tokens) — done
- Packaging: Docker Compose (self-host) + Helm chart (EKS/AKS/GKE) — done
- Entra ID OIDC SSO for the portal (identity → role mapping) — done
- Multi-tenancy + RBAC (tenant isolation; admin/reviewer/viewer) — done

## M7 — Enterprise config + multi-source

Make Bifrost ready for a large, regulated, multi-org estate — and prove the source seam is truly
platform-agnostic by adding two more sources beyond Azure DevOps.

- Connection model: ADO / GitHub / LLM connections by **secret reference** (vault refs default,
  AES-256-GCM encrypted fallback) — never raw secret values — done
- Portal config: connections + LLM routing policy (Copilot, Claude, OpenAI-compatible, local
  Gemma/Ollama) editable per tenant — done
- Many orgs per tenant: portfolio heatmap groups by org; org switcher — done
- First-run onboarding wizard (guided setup with live checks) — done
- **Jenkins** source adapter (`SourceAdapter` over the JSON API) — done
- **GitLab CI** source adapter (`SourceAdapter` over the v4 API) — done
- **Bitbucket Pipelines** source adapter (`SourceAdapter` over the Cloud v2 API) — done
- **CircleCI**, **Travis CI**, and **Bamboo** source adapters — done (every Importer-supported source now has an adapter)
- Source-adapter conformance suite (one contract, all seven platforms) — done

---

> **Where we are:** the core platform (M2–M6) is complete and runs end-to-end against live Azure
> DevOps projects — audit → convert → review → approve → PR → sandbox-validate → parity → a signed
> attestation and an org audit pack — deployable via Docker Compose or Helm, with Entra ID SSO,
> per-tenant isolation + RBAC, and least-privilege GitHub App auth. **M7** adds enterprise config
> (secret-reference connections, per-tenant LLM routing, many orgs per tenant, an onboarding
> wizard) and proves the source seam with adapters for **every Importer-supported source** — Jenkins,
> GitLab, Bitbucket, CircleCI, Travis, and Bamboo — behind a shared conformance suite, so Bifrost
> bridges all of them to GitHub Actions.
> Auth and multi-tenancy are opt-in, so the air-gapped single-box path is unchanged. Remaining work
> is the **M0/M1** foundations tail (devcontainer, licence/CONTRIBUTING, fixture harness, forecast
> wrap, version pinning, Entra-side ADO auth) and cross-cut hardening (prompt-eval harness, LLM
> cost tracking, observability, external-call resilience).
