# Bifrost — Azure DevOps → GitHub Actions migration showcase

**Companion transcript for `bifrost-showcase.mp4`** (1280×720, ~2 min 8 s).

Bifrost is an orchestration + intelligence layer on top of GitHub's official migration CLI
(`gh actions-importer`). It turns one-at-a-time, CLI-only pipeline conversion into a
portfolio-scale, semantically-reviewed, human-approved, attestable migration. It **wraps** the
official Importer — it never reimplements the conversion logic.

This recording is a **real** migration of a live Azure DevOps organisation
(`dev.azure.com/olaffreund0455`) to GitHub Actions. Everything shown was created during the
recording and is **public** — the links at the end are live; review them yourself.

---

## What was migrated

Three ADO projects, six YAML pipelines, converted by the Importer and reviewed in Bifrost. Each
pipeline carried at least one **gap** — a task the Importer cannot transform — which Bifrost
surfaces, explains, and routes to either a grounded LLM gap-fill or a manual-task runbook item.

| ADO project | Pipeline | Importer result | Gaps Bifrost surfaced |
|---|---|---|---|
| Contoso-Payments | Contoso-Payments-CI | Partial | `DownloadSecureFile@1` (code-signing .pfx), `SonarQubePrepare@5` |
| Contoso-Payments | Contoso-Payments-Release | Partial | `acme-corp.vault-tools.FetchSecret@2` (3rd-party) |
| Northwind-Logistics | Northwind-Logistics-CI | Partial | `Gradle@3`, `DownloadSecureFile@1` (release keystore) |
| Northwind-Logistics | Northwind-Logistics-Deploy | Partial | `KubernetesManifest@1` |
| Fabrikam-Identity | Fabrikam-Identity-CI | Partial | `DownloadSecureFile@1` (OIDC signing key), `WhiteSource@21` |
| Fabrikam-Identity | Fabrikam-Identity-Release | Partial | `contoso.deploy-tools.NotifyTeams@3` (3rd-party) |

The Importer reported **0 unsupported / 100 % partially successful** — i.e. it converted the bulk of
every pipeline, and the residue is exactly the high-risk, security-sensitive surface (secure files,
SAST, secret fetch, K8s deploy) that a human must review. That residue is the whole point of
Bifrost.

---

## Transcript

### 0:00 — Title
> **Bifrost — Azure DevOps to GitHub Actions, at portfolio scale.**
> Wrap the Importer. Review-first. Gap-aware. Attestable.

### 0:05 — Section 1 · Audit & convert (the CLI)
The official Importer does the bulk; Bifrost finds the gaps.

Commands shown (run live against the real ADO org):

```bash
# 1. Audit a real Azure DevOps organisation (read-only, ADO REST)
bifrost audit --project Contoso-Payments
#  -> discovers every project in the org and classifies each pipeline

# 2. The Importer converts the bulk — Bifrost surfaces what it can't
sed -n '17,32p' contoso-payments-ci.yml      # the converted GitHub Actions workflow
#  -> shows inline "# This item has no matching transformer" markers:
#     DownloadSecureFile@1, SonarQubePrepare@5  =>  typed Gaps

# 3. Reviewed in the portal, then committed as a pull request — never a silent write
gh pr list --repo olafkfreund/contoso-payments
```

Narration: *Every project in the org is discovered. Each `# no matching transformer` line is a
typed Gap. Bifrost routes each Gap to a grounded LLM explanation or a manual-task runbook item — it
never converts pipelines from scratch, and it never silently rewrites production CI.*

### 0:35 — Section 2 · Review (the portal)
A portfolio heatmap, a three-pane diff, and a program board — all live data.

- **Portfolio heatmap** — migration risk across 6 pipelines in 3 projects; 6 green / 0 amber / 0
  red; 100 % mechanically convertible; per-project risk tiles.
- **Program board** (Board / Roadmap / Issues) — an API-backed mirror of the GitHub Project Bifrost
  would stand up: one issue per pipeline, waves (Pilot / Early / Late majority), Bifrost-computed
  KPIs. Honest framing: *nothing is created on GitHub until you approve provisioning.*
- **Forecast / Readiness / Program** — runner-minute forecast, OIDC/secret readiness, wave plan.
- **Review — the three-pane diff** — original ADO YAML ↔ converted GitHub Actions workflow, with
  the **Risk** panel and the **Runbook** (the `DownloadSecureFile` gap becomes a checklist item:
  re-create the secure file as an encrypted GitHub Actions secret).

### 1:05 — Section 3 · The result (public on GitHub)
Bifrost opened the pull requests; everything is public.

- The **repository** (`contoso-payments`) — the original ADO pipelines under `azure-pipelines/`,
  and the Bifrost-converted workflow arriving as a reviewable PR.
- The **pull request diff** — `.github/workflows/contoso-payments-ci.yml`, with the gap comments
  preserved inline so a reviewer sees exactly what needs human attention. PR body records the risk
  band, the proposal id, and the manual-task checklist.
- The **other two repos** (`northwind-logistics`, `fabrikam-identity`) — same flow.
- The **program board** — `github.com/users/olafkfreund/projects/8`, one item per pipeline.

### 2:00 — Review it yourself
All artifacts are public (links below).

---

## Public artifacts (all live)

**Repositories** (each: original ADO source on `main` + a Bifrost-opened PR adding the converted workflow)
- https://github.com/olafkfreund/contoso-payments
- https://github.com/olafkfreund/northwind-logistics
- https://github.com/olafkfreund/fabrikam-identity

**Pull requests** (the converted workflows, opened by Bifrost)
- https://github.com/olafkfreund/contoso-payments/pull/1 · https://github.com/olafkfreund/contoso-payments/pull/2
- https://github.com/olafkfreund/northwind-logistics/pull/1 · https://github.com/olafkfreund/northwind-logistics/pull/2
- https://github.com/olafkfreund/fabrikam-identity/pull/1 · https://github.com/olafkfreund/fabrikam-identity/pull/2

**Program board**
- https://github.com/users/olafkfreund/projects/8

---

## What this proves

1. **Real, not staged.** A live ADO org was audited; the official Importer ran in Docker; six real
   workflows were converted; six real PRs were opened; a real GitHub Project was provisioned.
2. **Portfolio scale.** Three projects and six pipelines reviewed in one heatmap, one board, one
   forecast — not one pipeline at a time.
3. **Gap-aware.** Every pipeline's hard residue (secure files, SonarQube, WhiteSource, vault, Teams,
   Gradle, Kubernetes) was surfaced as a typed gap with a runbook item — never silently dropped.
4. **Review-first & attestable.** Conversions land as pull requests for human approval; the
   commit/provision paths are opt-in (`BIFROST_COMMIT_LIVE` / `BIFROST_BOARD_LIVE`) and every state
   transition is recorded — never a silent rewrite of production CI.

*Generated by Bifrost. The Importer converts; the human approves; Bifrost orchestrates, explains,
and attests.*
