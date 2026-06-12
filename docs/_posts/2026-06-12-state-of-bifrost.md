---
title: "State of Bifrost: what is built, what works, and how to use it"
layout: default
date: 2026-06-12
nav_exclude: true
description: "A grounded status report — the audit and review pipeline that works today, what is still ahead, and five real migration scenarios with the exact steps."
---

# State of Bifrost: what is built, what works, and how to use it
{: .fs-8 }

{{ page.date | date: "%B %-d, %Y" }}
{: .fs-3 .fw-300 .text-grey-dk-000 }

Bifrost is an orchestration and intelligence layer on top of GitHub's official migration
CLIs — `gh actions-importer` and GEI/`ado2gh`. It turns one-at-a-time, syntactic,
CLI-only pipeline conversion into a portfolio-scale, semantically reviewed, human-approved,
documented migration with a pluggable, air-gap-capable multi-model LLM layer. We wrap the
official tools; we never reimplement their conversion logic.

This post is a snapshot: what is built and working, what is still ahead, and how you would
actually use it on a real migration.

---

## What is built and working

The core thesis — assess a portfolio, convert with the official tools, fill the gaps with a
grounded model, and put a human in front of every change — runs end to end today.

**Portfolio audit and risk heatmap.** Point Bifrost at an Azure DevOps org and it runs the
Importer's `audit` across every pipeline, then renders a heatmap grouped by project and
coloured Green / Amber / Red. Classic (designer) pipelines — the hard tail — are classified
separately from YAML. The risk score is **deterministic**: it is computed from weighted,
explainable factors (container jobs, variable groups, service connections, multi-stage
gates), never from the model. The model explains and fills gaps; it does not score.

**Seven source platforms behind one adapter.** Azure DevOps is the first implementation of a
`SourceAdapter` trait; Jenkins, GitLab, CircleCI, Travis, and Bamboo follow the same contract,
with Bitbucket as a discovery-only source. A single conformance suite runs the trait contract
against all of them from captured fixtures.

**Grounded conversion and the three-pane review.** Per pipeline, Bifrost runs the Importer's
`dry-run`, parses the log into typed gap records (unsupported steps, partial constructs, manual
tasks), and builds a grounded request per gap — the source snippet, the Importer's converted
output, and the specific failure. The model fills the gap from that diff; it does not convert
from scratch. The result is a Proposal: the augmented workflow, a rationale, deterministic risk
flags, verify-before-approving steps, and a manual-task runbook. A reviewer sees the original
pipeline, the converted workflow with the model's gap-fills highlighted, and the rationale side
by side — and approves, requests changes, or edits inline before anything is committed.

**A multi-model, air-gap-capable LLM layer.** Orchestration calls a single `LlmProvider` trait,
never a vendor SDK. Implementations cover Anthropic, Google Gemini, GitHub Models/Copilot,
Azure OpenAI, Google Vertex, an OpenAI-compatible endpoint (including Bedrock gateways), and
local Ollama. A routing policy sends bulk and cheap work to a local or small model and hard
reasoning to a frontier model. Air-gap mode forces everything local and asserts zero external
calls.

**Change-management-grade reporting, PR-only delivery.** Before any change, Bifrost produces a
status report — per project, since different projects have different owners and boards — in both
Markdown and PDF, listing exactly what changes and what must be set up in GitHub (secrets to
create, variables, service connections to re-federate, the Actions allow-list). Nothing is
changed by generating it. When you do proceed, delivery is a pull request: Bifrost pushes a
branch and opens a PR; it refuses to commit a proposal that has not been approved.

**The portal.** A review-first control plane: the portfolio heatmap, the deterministic risk
breakdown, the three-pane review, the lifecycle queue, and Settings for connections and routing.
It ships two palettes (Gruvbox and a neutral shadcn-style) in light and dark, and its operator
docs travel inside the portal itself.

## What is still ahead

We are honest about the edges. The audit, conversion, review, reporting, and PR-delivery path
is solid; the work that remains is hardening and the compliance and deployment milestone.

- **Persistence in deployment.** The control plane runs with an in-memory store by default;
  wiring durable Postgres/SQLite persistence into every deployment (so audits and connections
  survive a restart) is in progress.
- **Validation parity.** Triggering the converted workflow in a sandbox and producing a parity
  report against the ADO baseline (status, artifact set, declared outputs — smoke parity, not
  full equivalence) is partially built.
- **Compliance and deploy.** Attestation export, the GitHub App with Entra ID OIDC,
  multi-tenancy, and Helm packaging are the next milestone.
- **The classic-pipeline tail.** Designer pipelines default to Amber/Red and need the most
  human review; improving their gap-fill is ongoing.
- **Backstage integration.** TechDocs publishing and catalog annotations, so a platform team can
  surface Bifrost inside their developer portal, are planned next.

## Five real scenarios, and how

### 1. Assess before you commit

A platform lead needs a portfolio-wide migration assessment for a change advisory board before
anyone touches production CI. Add an Azure DevOps connection on the Settings page (Bifrost stores
a reference to the secret — a Key Vault URI, a GitHub App, an Entra federation, or an env-var
name — never the value). Run the audit. Read the heatmap, then download the per-project PDF report
and hand it to each project's owner. No pipeline has been changed; the board approves on evidence.

### 2. Migrate one high-risk classic pipeline, reviewed

A team owns a Red, classic pipeline with a secure-file download and a custom marketplace task.
Open its proposal. The middle pane shows the converted workflow with the model's gap-fills
highlighted; the right pane explains each one, flags what a human must check, and lists the
manual-task runbook — for example, "this marketplace task has no first-party action; choose a
replacement" and "this service connection must be federated to GitHub via OIDC." Edit the
workflow inline if needed, approve, and Bifrost opens a pull request. The base branch is never
written to directly.

### 3. Run inside an air-gapped, regulated network

A bank can reach in-network model endpoints but not the public internet. Turn on air-gap mode.
Bifrost disables every frontier provider, routes all conversion through a local Ollama model or
an in-network endpoint, and the air-gap test target asserts that no pipeline data leaves the box.
Every state transition and human action is appended to an immutable audit log for attestation.

### 4. Consolidate a multi-source estate

An enterprise runs Azure DevOps, Jenkins, and GitLab. Add a connection for each on the Settings
page. Bifrost audits all three through the same Importer-backed adapters and merges them into one
portfolio, tagged by source org, so you triage and migrate a mixed estate from a single tool
instead of three migrations.

### 5. Keep conversion costs predictable at scale

Converting thousands of pipelines with a frontier model is expensive. Set a routing policy: bulk
and mechanical conversions go to a local or small model; only the hard reasoning and the
documentation go to a frontier model. You get frontier quality where it matters and local cost
everywhere else, with the same review gate in front of all of it.

---

## The non-negotiables, restated

Everything above sits on a few rules that do not bend. Review-first: production CI is never
silently rewritten; auto-commit is opt-in behind human approval and validation. Grounded
generation only: the model fills gaps from the diff, it does not convert from scratch. The LLM
explains; it does not score. Everything is attestable. And we wrap the official tools rather than
forking them — pinning and recording tool versions and image digests for every job so conversions
are reproducible.

If you are planning a portfolio-scale migration to GitHub Actions and you need it reviewed,
documented, and defensible, that is exactly what Bifrost is for.
