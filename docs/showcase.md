---
title: Showcase
nav_order: 4
---

# Migration showcase
{: .no_toc }

A **real** migration, end to end: a live Azure DevOps organisation — three projects, six YAML
pipelines — converted to GitHub Actions, reviewed in the Bifrost portal, and committed as pull
requests. Everything below was created during the recording and is **public** — review it yourself.

<video controls preload="metadata" width="100%" style="max-width:960px;border-radius:8px;border:1px solid #3c3836"
       src="{{ '/assets/showcase/bifrost-showcase.mp4' | relative_url }}">
  Your browser does not support embedded video.
  <a href="{{ '/assets/showcase/bifrost-showcase.mp4' | relative_url }}">Download the showcase (MP4)</a>.
</video>

About two minutes. Companion [transcript](https://github.com/olafkfreund/bifrost/blob/main/showcase/transcript.md)
and [voiceover script](https://github.com/olafkfreund/bifrost/blob/main/showcase/voiceover.md) on GitHub.

## What you are watching

1. **Audit & convert (CLI)** — a read-only audit of the real ADO org; the official Importer converts
   the bulk; Bifrost surfaces the gaps it cannot transform.
2. **Review (portal)** — a portfolio heatmap, the program board (Board / Roadmap / Issues), and the
   three-pane diff with deterministic risk and a runbook for each gap.
3. **The result (GitHub)** — three public repositories, six Bifrost-opened pull requests, and a
   public program board.

## The gaps Bifrost surfaced

The Importer reported 0 unsupported / 100% partially successful — it converted the bulk of every
pipeline, leaving exactly the security-sensitive residue a human must review. That residue is the
point of Bifrost.

| Pipeline | Gaps surfaced |
|---|---|
| Contoso-Payments-CI | `DownloadSecureFile@1` (code-signing .pfx), `SonarQubePrepare@5` |
| Contoso-Payments-Release | `acme-corp.vault-tools.FetchSecret@2` |
| Northwind-Logistics-CI | `Gradle@3`, `DownloadSecureFile@1` (release keystore) |
| Northwind-Logistics-Deploy | `KubernetesManifest@1` |
| Fabrikam-Identity-CI | `DownloadSecureFile@1` (OIDC signing key), `WhiteSource@21` |
| Fabrikam-Identity-Release | `contoso.deploy-tools.NotifyTeams@3` |

## Public artifacts

| Project | Repository | Pull requests |
|---|---|---|
| Contoso-Payments | [contoso-payments](https://github.com/olafkfreund/contoso-payments) | [#1](https://github.com/olafkfreund/contoso-payments/pull/1) · [#2](https://github.com/olafkfreund/contoso-payments/pull/2) |
| Northwind-Logistics | [northwind-logistics](https://github.com/olafkfreund/northwind-logistics) | [#1](https://github.com/olafkfreund/northwind-logistics/pull/1) · [#2](https://github.com/olafkfreund/northwind-logistics/pull/2) |
| Fabrikam-Identity | [fabrikam-identity](https://github.com/olafkfreund/fabrikam-identity) | [#1](https://github.com/olafkfreund/fabrikam-identity/pull/1) · [#2](https://github.com/olafkfreund/fabrikam-identity/pull/2) |

Program board: [github.com/users/olafkfreund/projects/8](https://github.com/users/olafkfreund/projects/8)

## How it was built

The scripts that produced this recording — terminal cast, live-portal and GitHub Playwright
walkthroughs, repo/PR creation, and ffmpeg stitching — are in
[`showcase/`](https://github.com/olafkfreund/bifrost/tree/main/showcase) in the repository.
Conversions land as pull requests for human approval; the commit and board-provisioning paths are
opt-in (`BIFROST_COMMIT_LIVE` / `BIFROST_BOARD_LIVE`) — never a silent rewrite of production CI.
