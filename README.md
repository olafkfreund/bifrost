<div align="center">

# Bifrost

**The bridge between worlds — Azure DevOps · Jenkins · GitLab → GitHub Actions, at portfolio scale.**

Bifrost is an orchestration + intelligence layer on top of GitHub's official migration CLIs
(`gh actions-importer`, GEI/`ado2gh`). It turns one-at-a-time, syntactic, CLI-only pipeline
conversion into a **portfolio-scale, semantically-reviewed, human-approved, fully-documented**
migration — with a pluggable, **air-gap-capable** multi-model LLM layer.

[**Documentation & Showcase**](https://bitfrost.freundcloud.com/)

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
![Status: Active development](https://img.shields.io/badge/status-M2--M6%20complete-brightgreen)
![Built on](https://img.shields.io/badge/wraps-gh%20actions--importer%20%C2%B7%20GEI-2088FF)
![Editor](https://img.shields.io/badge/MCP-VS%20Code%20·%20Cursor%20·%20Claude-5A4FCF)

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
- **Works in your editor.** A built-in [MCP server](https://bitfrost.freundcloud.com/mcp) lets an
  agent in VS Code (or Cursor / Claude Desktop / the Copilot coding agent) query the portfolio and
  convert a pipeline to a proposed workflow — review-first, with commit triple-gated. Open a legacy
  `azure-pipelines.yml`, ask once, get a reviewable GitHub workflow with its risks and manual
  follow-ups spelled out.
- **Program on GitHub itself.** A dry-run plan for a dedicated repo + org Project (board / roadmap /
  KPIs), so the migration is tracked and reported on GitHub's own golden-path features.

## Architecture at a glance

```
PORTAL (React/TS)           portfolio heatmap · 3-pane diff · approve/edit · runbook · board
      │ REST/SSE                                      ▲ REST
      │                            bifrost-mcp (MCP/stdio) ── VS Code · Cursor · Claude · Copilot
CONTROL PLANE (Rust/axum)   job state machine · conversion orchestrator ·
      │                     deterministic risk model · attestation + audit log
      │ LlmProvider trait → Anthropic · Gemini · Copilot/Models · Azure OpenAI ·
      │                     Vertex AI · Bedrock (gateway) · OpenAI-compat · Ollama (air-gap)
      ▼ shell-out (Docker)            ▼ HTTP
INGESTION ADAPTERS          EXTERNAL: ADO REST API · GitHub API · GEI
  gh actions-importer (Docker) · SourceAdapter trait (ADO → …)
```

Full design: [`bifrost-implementation-plan.md`](bifrost-implementation-plan.md) ·
the [architecture doc](https://bitfrost.freundcloud.com/architecture) (diagrams, flowchart,
design rationale) · rendered on the [docs site](https://bitfrost.freundcloud.com/).

## Roadmap

| Milestone | Focus | Status |
|---|---|---|
| **M0** Foundations | Workspace, CI, devcontainer, licence, docs, fixtures | In progress |
| **M1** Audit MVP | ADO adapter + Importer audit wrapper + portfolio heatmap | Mostly done |
| **M2** Conversion + LLM | dry-run wrapping, gap detection, LLM layer, risk model | Done |
| **M3** Review Portal | Three-pane diff, approve/edit, proposal lifecycle | Done |
| **M4** Commit + PR | Push/migrate, manual-task checklists, PR automation | Done |
| **M5** Validation | Sandbox trigger + run capture + parity diff | Done |
| **M6** Compliance + Deploy | Attestation, audit pack, App auth, packaging, Entra SSO, multi-tenancy + RBAC | Done |

The end-to-end loop works against live Azure DevOps projects: audit a portfolio, convert a
pipeline (real Importer dry-run + grounded LLM gap-fill, air-gap capable), review and approve in
the portal, open a PR, trigger the converted workflow in a sandbox, capture its run, and diff it
against the ADO baseline for smoke parity — then export a **signed, in-toto-inspired attestation**
and a per-org **audit pack**. Deployable via [Docker Compose or Helm](deploy/), with **Entra ID
SSO**, **per-tenant isolation + RBAC**, and a least-privilege **GitHub App** — all opt-in, so the
air-gapped single-box path stays simple. The core platform (M2–M6) is complete; the M0/M1
foundations tail remains.

Most recent: a built-in **MCP server** brings the whole flow into the editor — `bifrost_convert`
(convert a pipeline to a proposed workflow), `bifrost_runbook` (read the manual-task checklist),
and a triple-gated `bifrost_commit` (open the PR for an approved proposal) — plus a **program board**
that plans a dedicated repo + org Project (board / roadmap / KPIs) on GitHub itself. See the
[editor guide](https://bitfrost.freundcloud.com/mcp) and the
[architecture doc](https://bitfrost.freundcloud.com/architecture).

## Run it (self-host)

The fastest way to a running Bifrost is Docker Compose — SQLite-backed and **air-gap by
default**, so nothing leaves the box:

```bash
cd deploy
docker compose up --build      # portal + API on http://localhost:8080
```

That's the whole product: open the portal, add a source connection, audit a portfolio, and
review conversions. Set `BIFROST_SIGNING_KEY` in production (attestation signing), and switch
to Postgres with `--profile postgres` for the multi-tenant server. Full options — including
**Entra ID SSO**, RBAC, and the **Helm** chart for Kubernetes — are in [`deploy/`](deploy/).

To drive it from your editor instead, point VS Code (or Cursor / Claude Desktop) at the MCP
server — see the [editor guide](https://bitfrost.freundcloud.com/mcp).

> A [release workflow](.github/workflows/release.yml) publishes **cosign-signed** images with
> **SPDX SBOM attestations** to GHCR (`ghcr.io/olafkfreund/bifrost-{api,portal}`) on each
> tagged release — pull and `cosign verify` instead of building. See
> [`deploy/`](deploy/#images) for the pull + verify recipe. Until the first tag is cut, build
> from source with the command above.

## Getting started (contributors)

On NixOS (or any machine with Nix + flakes), the dev shell provides the whole
toolchain — the pinned Rust toolchain, Node 22, `gh`, `azure-cli`, and the Docker
client:

```bash
nix develop                   # enter the Bifrost dev shell (honours rust-toolchain.toml)
cargo test --workspace        # full suite green (100+ tests)
cd portal && npm ci && npm run dev   # portal (mock data) at http://localhost:5173
```

The GitHub Actions Importer isn't in nixpkgs (it's a `gh` extension + Docker
image); install it once into your writable home:

```bash
gh extension install github/gh-actions-importer
gh actions-importer configure   # uses GITHUB_TOKEN + AZDO_PAT
```

Tokens (ADO PAT, GitHub token) live in a gitignored `.envrc` — `source .envrc`.
Work is issue-driven and milestone-ordered; see [`CONTRIBUTING.md`](CONTRIBUTING.md)
for the workflow (Conventional Commits, one PR per issue) and [`CLAUDE.md`](CLAUDE.md)
for the architecture and hard rules.

## Stack (target)

Rust (axum · tokio · sqlx) · Postgres / SQLite · official Importer Docker image ·
React + TS + Vite + Tailwind + Monaco · GitHub App + Entra ID OIDC · Docker Compose → Helm.

## License

[MIT](LICENSE) © 2026 Olaf K. Freund
