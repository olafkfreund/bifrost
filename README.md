<div align="center">

# 🌈 Bifrost

**The bridge between worlds — Azure DevOps → GitHub Actions, at portfolio scale.**

Bifrost is an orchestration + intelligence layer on top of GitHub's official migration CLIs
(`gh actions-importer`, GEI/`ado2gh`). It turns one-at-a-time, syntactic, CLI-only pipeline
conversion into a **portfolio-scale, semantically-reviewed, human-approved, fully-documented**
migration — with a pluggable, **air-gap-capable** multi-model LLM layer.

[**📖 Documentation & Showcase →**](https://olafkfreund.github.io/bifrost/)

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
![Status: Planning](https://img.shields.io/badge/status-planning%20(M0)-yellow)
![Built on](https://img.shields.io/badge/wraps-gh%20actions--importer%20%C2%B7%20GEI-2088FF)

</div>

---

## The thesis

> We do **not** rebuild repo migration or YAML translation. GitHub already does ~90% of the
> syntactic conversion. Bifrost owns the **other 10%, the review/approval workflow, the portfolio
> orchestration, semantic-equivalence validation, and the audit trail** — the parts Microsoft is
> structurally disincentivised to build well.

We **wrap** the official tools; we never reimplement their conversion logic.

## What makes it different

- **Review-first, not autonomous.** Bifrost recommends and explains; it never silently rewrites
  production CI. Auto-commit is opt-in, gated behind human approval + validation.
- **Air-gap capable.** Run with a local model only (Ollama / llama.cpp) so no pipeline definition
  ever leaves your network. Pipeline YAML leaks infra topology and secret *names* — keep it home.
- **The LLM explains; it does not score.** Risk scoring is deterministic and explainable. The
  model fills gaps, explains, and flags — it never produces the numeric risk score.
- **Grounded generation only.** Every LLM request carries the source snippet + the Importer's
  converted output + the specific failure. The model fills the gap from that diff — it never
  converts a pipeline from scratch.
- **Attestation-native.** Every decision — who approved what, what changed, why, the validation
  result — is recorded as a signed, exportable attestation.

## Architecture at a glance

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

Full design: [`bifrost-implementation-plan.md`](bifrost-implementation-plan.md) ·
rendered on the [docs site](https://olafkfreund.github.io/bifrost/).

## Roadmap

| Milestone | Focus |
|---|---|
| **M0** Foundations | Workspace, CI, devcontainer, licence, docs, fixtures |
| **M1** Audit MVP | ADO adapter + Importer audit wrapper + portfolio heatmap |
| **M2** Conversion + LLM | dry-run wrapping, gap detection, LLM layer, risk model |
| **M3** Review Portal | Three-pane diff, approve/edit, proposal lifecycle |
| **M4** Commit + PR | Push/migrate, manual-task checklists, PR automation |
| **M5** Validation | Sandbox trigger + parity report |
| **M6** Compliance + Deploy | Attestation export, auth, multi-tenant, Helm |

## Getting started (contributors)

This repo is **pre-implementation**. Work is issue-driven and milestone-ordered:

```bash
gh auth login                 # authenticate the GitHub CLI
./seed-issues.sh              # bootstrap milestones, labels, epics, issues (run ONCE)
```

Then pick up the first M0 issue ("Scaffold Cargo workspace") and work one epic at a time.
See [`CLAUDE.md`](CLAUDE.md) for the full contributor workflow and hard rules.

## Stack (target)

Rust (axum · tokio · sqlx) · Postgres / SQLite · official Importer Docker image ·
React + TS + Vite + Tailwind + Monaco · GitHub App + Entra ID OIDC · Docker Compose → Helm.

## License

[MIT](LICENSE) © 2026 Olaf K. Freund
