# Bifrost

Bifrost is an orchestration and intelligence layer on top of GitHub's official
migration CLIs (`gh actions-importer` and GEI/`ado2gh`). It turns one-at-a-time,
syntactic, CLI-only pipeline conversion (Azure DevOps and other CI/CD systems to
GitHub Actions) into a portfolio-scale, semantically reviewed, human-approved,
documented migration with a pluggable, air-gap-capable multi-model LLM layer.

We wrap the official tools; we never reimplement their conversion logic.

## What it does

- **Audits** an entire portfolio with the Importer and renders a risk heatmap,
  grouped by project and coloured Green / Amber / Red.
- **Converts** each pipeline with the Importer's dry-run, then fills the gaps the
  Importer could not with a grounded LLM request.
- **Reviews** every change in a three-pane diff before anything is committed.
- **Reports** what will change and what must be set up in GitHub, per project, in
  Markdown and PDF, for a change advisory board.
- **Delivers** as a pull request, never a silent rewrite of production CI.

## The non-negotiables

These rules do not bend:

- **Review-first.** Production CI is never silently rewritten. Auto-commit is
  opt-in, gated behind human approval and validation.
- **Air-gap capable.** A customer can run with a local model only; in air-gap mode
  frontier providers are disabled and no pipeline data leaves the box. There is an
  explicit test target asserting zero external calls.
- **The LLM explains; it does not score.** Risk scoring is deterministic. The model
  fills gaps, explains, and flags — it never produces the numeric risk score.
- **Grounded generation only.** Each LLM request carries the source snippet, the
  Importer's converted output, and the specific failure. The model fills the gap
  from that diff; it does not convert pipelines from scratch.
- **Everything is attestable.** Every state transition and human action is appended
  to an immutable audit log.
- **Wrap, don't fork.** We shell out to the official tools and pin and record tool
  versions and image digests for every job, so conversions are reproducible.

## Glossary

- **Importer** — `gh actions-importer` (audit / forecast / dry-run / migrate).
- **GEI / ado2gh** — repo migration tooling (history, branches, metadata).
  Out of scope for conversion; orchestrated and tracked only.
- **Gap** — a construct the Importer marked partial or unsupported, routed to the LLM.
- **Proposal** — an augmented workflow plus rationale and risk, awaiting human review.
- **Parity report** — a smoke-validation result comparing a converted workflow run to
  the ADO baseline (status, artifact set, declared outputs).
- **Classic pipeline** — an ADO designer pipeline (the hard tail, defaults Amber/Red);
  a YAML pipeline is the easy path.
