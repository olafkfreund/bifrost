# Roadmap

Milestone order is the build sequence. Status reflects the current state of the
repository; see the GitHub milestones and epics for detail.

## M0 — Foundations

Cargo workspace, CI (format, clippy, build, test), devcontainer, MIT licence, docs
site, Importer-output fixtures. **Delivered.**

## M1 — Audit MVP

`SourceAdapter` trait, `AzureDevOpsAdapter`, the Importer `audit` wrapper, and the
portfolio heatmap. This slice alone demonstrates the thesis. **Delivered.**

## M2 — Conversion and LLM

`dry-run` wrapping, gap detection, the `LlmProvider` trait with multiple
implementations (Anthropic, Gemini, Copilot/Models, Azure OpenAI, Vertex,
OpenAI-compatible, Ollama), the deterministic risk model, and the axum API with
persistence and job orchestration. **Delivered.**

## M3 — Review portal

The three-pane diff, approve/edit, and the proposal lifecycle. **Delivered.**

## M4 — Commit and PR

Push/`migrate`, manual-task checklists, and PR automation. The publisher opens a pull
request and refuses to commit an unapproved proposal. **In progress** (hardening).

## M5 — Validation

Sandbox `workflow_dispatch` trigger and a parity report against the ADO baseline
(smoke parity: status, artifact set, declared outputs). **In progress.**

## M6 — Compliance and deploy

Attestation export, the GitHub App with Entra ID OIDC, multi-tenancy, and Helm
packaging. **Ahead.**

## Cross-cutting work delivered alongside the milestones

- Seven source platforms behind the `SourceAdapter` trait, with a single conformance
  suite (Jenkins, GitLab, CircleCI, Travis, Bamboo, plus discovery-only Bitbucket).
- Enterprise cloud LLM providers (Azure OpenAI, Google Vertex, Bedrock gateway).
- A change-management-grade status report, per project, in Markdown and PDF.
- Portal polish: GitHub typefaces, an elevation and dialog system, a left-sidebar
  navigation, and a Gruvbox / shadcn theme system.

## Known remaining work

- Durable persistence wired into every deployment (so audits and connections survive
  a restart).
- The classic-pipeline tail (designer pipelines need the most human review).
- Live model discovery and interactive OAuth acquisition flows.
- This Backstage TechDocs and catalog integration (in place; iterating).
