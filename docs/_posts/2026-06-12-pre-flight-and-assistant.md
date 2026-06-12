---
title: "From audit to pre-flight: forecast, coverage, assessment, readiness, and an assistant"
layout: default
date: 2026-06-12 19:00:00 +0100
nav_exclude: true
description: "Closing the bookends of the golden path — what GitHub will cost, what's left behind, the source assessment, target readiness, and a grounded chat assistant."
---

# From audit to pre-flight: forecast, coverage, assessment, readiness, and an assistant
{: .fs-8 }

{{ page.date | date: "%B %-d, %Y" }}
{: .fs-3 .fw-300 .text-grey-dk-000 }

We measured Bifrost against GitHub's documented golden path for Azure DevOps to GitHub
Actions migration — `configure → audit → forecast → dry-run → migrate`, and the wider
Plan, Assess, Test, Migrate, Validate, Decommission program. Bifrost was already ahead on
conversion quality and governance: review-first, grounded generation, a deterministic risk
model, per-project reporting, and PR-only delivery. The gaps were on the **bookends** — the
*forecast* (what GitHub will cost and need) and the *program scaffolding* a migration manager
needs *before* changing anything.

This round closes those gaps. Every new surface follows the same rule the risk model does:
**the model explains; it does not score, and it does not price.** Cost, coverage, and
readiness are computed deterministically from the audit — never by an LLM.

## Forecast — what GitHub will cost and need

The golden path runs `gh actions-importer forecast` to project Actions usage before migrating.
Bifrost makes that projection deterministic: monthly and annual cost, runner-minutes, and a
per-project breakdown, computed from the audit's per-pipeline minutes against a configurable,
cited runner-rate table. Capacity figures that need real run history — peak concurrency, queue
time, duration percentiles — are *carried* from the Importer forecast, never fabricated.

![Forecast — projected GitHub Actions cost and capacity for the target org]({{ '/assets/screenshots/forecast-cost.png' | relative_url }})

## Coverage — find every moving part, leave nothing behind

"How do we make sure everything migrates?" is answered by a completeness matrix: every Azure
DevOps moving part mapped to its GitHub equivalent and a status — automatic, review, manual
setup, or **not yet inventoried**. That last status is the point: categories Bifrost cannot yet
enumerate (secure files, task groups, agent pools, environments and gates, artifact feeds) are
flagged in red rather than quietly omitted, so nothing is dropped without a trace.

![Coverage matrix — every ADO moving part mapped to its GitHub equivalent and status]({{ '/assets/screenshots/coverage-matrix.png' | relative_url }})

## Assessment — status of the source, before any change

The source side gets its own assessment: pipeline mix (YAML vs the classic designer tail), risk
distribution, and the **inventory density** a program must account for — service connections,
variable groups, secrets, self-hosted runners, custom task types, the Actions allow-list — with a
per-project breakdown. Signals Bifrost does not yet collect (dormancy, success-rate baseline,
owning team, repository size for GEI) are listed honestly so a team knows what is still unmeasured.

![Assessment — source mix, risk, and inventory density]({{ '/assets/screenshots/assessment.png' | relative_url }})

## Readiness — is GitHub ready to receive it

The pre-flight partner to the assessment. A checklist of what must be true in the target org:
runners sized to the forecast, the Actions allow-list, OIDC/Entra federation for service
connections (it even flags GitHub's 2026 change to the OIDC subject claim), secret management,
branch rulesets, ownership, and a rollback plan. Items Bifrost can quantify from the audit carry a
count and an action; operational gates it cannot verify are marked *unverified* rather than a
false green.

![Readiness — the target GitHub pre-flight checklist]({{ '/assets/screenshots/readiness.png' | relative_url }})

## An assistant that knows your migration

A popout assistant answers questions about the migration — cost, risk, coverage, a specific
project — grounded in the same deterministic state. It is routed through whatever LLM is
configured, so **air-gap mode forces it local** and no pipeline data leaves the box. It is
query-only by design: it explains and advises, but it cannot change anything. Control actions stay
behind the existing human-approval and audit-logged path.

![The migration assistant, grounded in the portfolio]({{ '/assets/screenshots/chat-assistant.png' | relative_url }})

## Routing and air-gap, unchanged in spirit

All of it respects the routing policy: bulk and cheap work to a local or small model, hard
reasoning to a frontier model, and in air-gap mode everything is forced local.

![LLM routing policy and the air-gap toggle]({{ '/assets/screenshots/routing-airgap.png' | relative_url }})

---

With these, the picture before a migration is complete: **Assessment** (what's in Azure DevOps),
**Forecast** (what GitHub will cost), **Coverage** (what migrates and what's left), and
**Readiness** (whether GitHub is ready) — and an assistant to ask about any of it. All of it
review-first, grounded, deterministic where it must be, and attestable. See the full
[golden-path alignment]({{ '/golden-path' | relative_url }}) for how this maps to GitHub's
documented process.

<small>Screenshots show Bifrost across its Gruvbox themes, against both the synthetic
<code>contoso</code> sample and a small live Azure DevOps audit. No secrets are shown — Bifrost
records secret names and types only, never values.</small>
