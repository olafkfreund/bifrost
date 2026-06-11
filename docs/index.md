---
title: Home
layout: default
nav_order: 1
description: "Bifrost — Azure DevOps → GitHub Actions migration at portfolio scale, semantically reviewed, human-approved, air-gap capable."
permalink: /
---

# Bifrost
{: .fs-9 }

The bridge between worlds. Azure DevOps → GitHub Actions migration at **portfolio scale** —
semantically reviewed, human-approved, fully documented, and **air-gap capable**.
{: .fs-6 .fw-300 }

[Read the plan](plan){: .btn .btn-primary .fs-5 .mb-4 .mb-md-0 .mr-2 }
[See it in action](screenshots){: .btn .fs-5 .mb-4 .mb-md-0 .mr-2 }
[View on GitHub](https://github.com/olafkfreund/bifrost){: .btn .fs-5 .mb-4 .mb-md-0 }

---

[![The Bifrost portfolio heatmap — migration risk across an Azure DevOps org]({{ '/assets/screenshots/portfolio-heatmap.png' | relative_url }})](screenshots)

<small>The portfolio heatmap — migration risk across an Azure DevOps org, at a glance.
[See more of the portal →](screenshots)</small>

---

## The thesis

GitHub's official tools (`gh actions-importer`, GEI/`ado2gh`) already do ~90% of the **syntactic**
conversion, one pipeline at a time. Bifrost owns the part nobody else does well:

> the **other 10%**, the review/approval workflow, the portfolio orchestration,
> semantic-equivalence validation, and the audit trail.

**We wrap the official tools; we never reimplement their conversion logic.**

## Why it's different

| | |
|---|---|
| **Review-first** | Recommends and explains; never silently rewrites production CI. Auto-commit is opt-in, gated behind human approval + validation. |
| **Air-gap capable** | Run with a local model only (Ollama / llama.cpp). No pipeline definition — which leaks infra topology and secret *names* — ever leaves your network. |
| **Deterministic risk** | Risk scoring is computed from explainable factors, not the LLM. The model **explains; it does not score**. |
| **Grounded generation** | Every LLM request carries the source snippet + the Importer's output + the specific failure. The model fills the gap from that diff — never converts from scratch. |
| **Attestation-native** | Every decision — who approved what, what changed, why, the validation result — is a signed, exportable attestation. |
| **Platform-agnostic** | The source adapter is an interface. ADO is the first implementation; Jenkins, GitLab, Bamboo, and others follow. |

## How it works

```
PORTAL (React/TS)           portfolio heatmap · 3-pane diff · approve/edit · runbook
      │ REST/SSE
CONTROL PLANE (Rust/axum)   job state machine · conversion orchestrator ·
      │                     deterministic risk model · attestation + audit log
      │ LlmProvider trait → Anthropic · Gemini · Copilot/Models · Ollama (air-gap)
      ▼ shell-out (Docker)            ▼ HTTP
INGESTION ADAPTERS          EXTERNAL: ADO REST API · GitHub API · GEI
  gh actions-importer (Docker) · SourceAdapter trait (ADO → …)
```

The **core loop**, per pipeline: Importer `dry-run` → parse the log for unsupported steps,
partial constructs, and manual tasks into typed **Gaps** → send each gap to the LLM with full
grounding → assemble an augmented workflow + rationale + risk → persist as a **Proposal** awaiting
human review. Nothing reaches production without a person approving it.

## Where it is

Bifrost ships in seven milestones. The full conversion loop already runs against live Azure
DevOps projects — **M2 (Conversion + LLM)**, **M3 (Review Portal)**, and **M4 (Commit + PR)**
are complete, and **M5 (Validation)** is underway:

| Milestone | Status |
|---|---|
| **M0** Foundations — workspace, CI, dev env, docs, fixtures | In progress |
| **M1** Audit MVP — ADO adapter + Importer audit + heatmap | Mostly done |
| **M2** Conversion + LLM — gap detection, LLM layer, risk model | Done |
| **M3** Review Portal — three-pane diff, approve/edit, lifecycle | Done |
| **M4** Commit + PR — push/migrate, runbooks, PR automation | Done |
| **M5** Validation — sandbox trigger, run capture, parity diff | In progress |
| **M6** Compliance + Deploy — attestation, auth, multi-tenant, Helm | Planned |

Today you can audit a portfolio, convert a pipeline (real Importer dry-run + grounded LLM
gap-fill, air-gap capable), review and approve it in the portal, open a PR, then trigger the
converted workflow in a sandbox, capture its run, and diff it against the ADO baseline.

[See the full roadmap](roadmap){: .btn .btn-outline }

---

<small>Bifrost is in active development. MIT licensed. Built on top of GitHub's official
migration CLIs.</small>
