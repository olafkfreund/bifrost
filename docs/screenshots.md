---
title: Screenshots
layout: default
nav_order: 2
description: "The Bifrost portal — portfolio heatmap, deterministic risk, the three-pane review, and the review queue."
permalink: /screenshots
---

# The portal
{: .fs-9 }

Bifrost's control plane is driven from a review-first portal: a portfolio heatmap of
migration risk, a deterministic risk breakdown per pipeline, a three-pane review where a
human approves or edits the converted workflow, and a queue that tracks every pipeline
through the lifecycle.
{: .fs-6 .fw-300 }

---

## Portfolio heatmap

Point Bifrost at an Azure DevOps org and get a portfolio-scale heatmap of migration risk —
grouped by project, coloured Green / Amber / Red, with the converted-ratio and risk score on
every tile. Classic (designer) pipelines — the hard tail — are flagged distinctly. The header
shows the pinned Importer version and whether **air-gap mode** is active.

![Bifrost portfolio heatmap — pipelines grouped by project and coloured by migration risk]({{ '/assets/screenshots/portfolio-heatmap.png' | relative_url }})

## Deterministic risk, explained

Click any pipeline to see *why* it scored the way it did. The score is computed from
**weighted, explainable factors** — container jobs, variable groups, service connections,
multi-stage gates — **never from the LLM**. The model explains and fills gaps; it does not
score.

![Risk-factor side panel — the deterministic score broken down into weighted factors]({{ '/assets/screenshots/risk-factors-panel.png' | relative_url }})

## Table view

The same portfolio as a dense, sortable table — type, risk, converted-ratio, manual tasks,
review status, and forecast runner-minutes per pipeline.

![Portfolio table view — every pipeline with risk, converted-ratio, status, and forecast minutes]({{ '/assets/screenshots/portfolio-table.png' | relative_url }})

## The three-pane review

The heart of the review-first workflow. **Left:** the original ADO pipeline. **Middle:** the
converted GitHub Actions workflow, with the gaps the LLM filled highlighted. **Right:** the
rationale, the deterministic risk flags, the verify-before-approving steps, the manual-task
runbook, and the immutable audit trail. A reviewer approves, or edits the workflow inline,
before anything is committed.

![Three-pane proposal review — source pipeline, converted workflow, and rationale + risk + runbook]({{ '/assets/screenshots/proposal-review.png' | relative_url }})

Edits are made in place and recorded — the audit trail captures who changed what, and why.

![Editing the converted workflow inline before approval]({{ '/assets/screenshots/proposal-edit.png' | relative_url }})

## Review queue

Every pipeline, tracked through the proposal lifecycle —
`draft → in_review → approved → committed → validated` — with a migration-progress bar and the
last action (who, when) on each row.

![Review queue — pipelines tracked through the proposal lifecycle with a progress bar]({{ '/assets/screenshots/review-queue.png' | relative_url }})

## Docs &amp; help, in the portal

Operator documentation ships inside the portal itself — getting started, using the portal, and
connecting to live data — so it travels with the tool, air-gapped or not.

![In-portal Docs & Help page]({{ '/assets/screenshots/docs-help.png' | relative_url }})

---

<small>Screenshots show Bifrost running against synthetic enterprise sample data
(`contoso`). No real pipeline definitions or secrets are shown.</small>
