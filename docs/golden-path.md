---
title: Golden path
layout: default
nav_order: 4
permalink: /golden-path
description: "How Bifrost maps to GitHub's documented golden path for Azure DevOps to GitHub Actions migration — and where it goes further."
---

# Following GitHub's golden path
{: .fs-9 }

GitHub publishes a recommended path for migrating Azure DevOps to GitHub. Bifrost follows
it step for step, and goes further where the documented path stops at "inspect the output
manually."
{: .fs-6 .fw-300 }

---

## The documented path

GitHub's migration is **two tools, one program**.

**Pipelines** are migrated with the GitHub Actions Importer (`gh actions-importer`):

| Step | What it does |
|------|--------------|
| `configure` | Authenticate to Azure DevOps and GitHub. |
| `audit` | Convert every pipeline in the org to assess feasibility; produce `audit_summary.md` categorising each pipeline as successful, partially successful, unsupported, or failed. |
| `forecast` | Compute historical usage (execution time, queue time, concurrency) to project GitHub Actions runner needs and cost. |
| `dry-run` | Convert one pipeline locally, without opening a pull request, so it can be refined. |
| `migrate` | Convert and open a pull request, with a "manual steps" checklist for what a human must finish. |

**Repositories** are migrated with the GitHub Enterprise Importer (GEI / `ado2gh`): git history,
branches, tags, and pull requests. GEI does not migrate pipelines, and the Importer does not
migrate repositories — they are independent, coordinated processes.

The overall program GitHub documents runs: **Plan, Assess, Test, Migrate, Validate and
Stabilize, Decommission.**

## Where Bifrost sits on the path

Bifrost wraps the official tools — it never reimplements their conversion logic — and pins and
records the tool version and image digest for every job, so conversions are reproducible.

| Golden-path element | Bifrost |
|---------------------|---------|
| `audit` | A portfolio heatmap grouped by project, coloured by deterministic risk, instead of a flat report. |
| `forecast` | A deterministic cost and capacity projection for the target org — monthly and annual cost, runner-minutes, and per-project breakdown. |
| `dry-run` | Typed gap records, with each gap filled by a grounded model request (the source snippet, the Importer's output, and the specific failure). |
| `migrate` | Delivery is a pull request, gated on human approval; the base branch is never written to directly. |
| Inspect for correctness | A three-pane review — original pipeline, converted workflow with gap-fills highlighted, and the rationale, risk flags, verify steps, and manual-task runbook. |
| Manual steps | A per-project change-management report (Markdown and PDF) listing the secrets, variables, service connections, environments, and the Actions allow-list to set up in GitHub. |

## Where Bifrost goes further

The documented path stops at "inspect each converted workflow for correctness before using it
as a production workload" — validation is manual and user-defined, and there is no formal parity
framework. Bifrost adds the governance an enterprise migration needs:

- **Review-first by default.** Production CI is never silently rewritten; auto-commit is opt-in,
  behind human approval and validation.
- **Deterministic, explainable risk and cost.** Both are computed from weighted, auditable
  factors — never from the model. The model explains and fills gaps; it does not score, and it
  does not price.
- **Grounded generation only.** Each model request carries the real context, so the model fills
  the gap from the diff rather than converting from scratch.
- **Attestable.** Every state transition and human action is appended to an immutable audit log.
- **Air-gap capable.** A customer can run with a local model only; in air-gap mode frontier
  providers are disabled and no pipeline data leaves the box.
- **Parity validation.** Smoke-validating a converted workflow against the Azure DevOps baseline
  (status, artifact set, declared outputs) — a step the documented path leaves to the reader.

## What still needs a human

The Importer documents constructs it cannot convert, and Bifrost surfaces them as manual tasks
rather than pretending they migrated: classic (designer) pipelines, pre- and post-deployment
gates and approvals, secrets and secure files (names only — values are never read), service
connections and their OIDC federation, environments, self-hosted runners, Azure Artifacts feeds,
and custom or marketplace tasks with no first-party Action. The per-project report and the runbook
make these explicit so nothing is silently dropped.

---

## Sources

- [Migrating from Azure DevOps with GitHub Actions Importer](https://docs.github.com/en/actions/tutorials/migrate-to-github-actions/automated-migrations/azure-devops-migration)
- [Automating migration with GitHub Actions Importer](https://docs.github.com/en/actions/tutorials/migrate-to-github-actions/automated-migrations/use-github-actions-importer)
- [Understand migrations from Azure DevOps to GitHub](https://docs.github.com/en/migrations/ado/understand-migrations-from-azure-devops-to-github)
- [Migrate your repositories from Azure DevOps to GitHub](https://docs.github.com/en/migrations/ado/migrate-your-repositories-from-azure-devops-to-github)
- [Azure DevOps to GitHub Enterprise migration guide (GitHub Well-Architected)](https://wellarchitected.github.com/library/scenarios/migrations/azure-devops-migration-guide/)
- [Migrate from Azure DevOps to GitHub (Microsoft Learn)](https://learn.microsoft.com/en-us/training/modules/introduction-to-ado-to-github-migration/)
