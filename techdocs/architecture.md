# Architecture

Bifrost is organised into three planes.

```
PORTAL (React/TS)           portfolio heatmap, 3-pane diff, approve/edit, runbook
      | REST/SSE
CONTROL PLANE (Rust/axum)   job state machine (PG/SQLite), conversion orchestrator,
      |                     deterministic risk model, attestation + audit log
      | LlmProvider trait -> Anthropic, Gemini, Copilot/Models, Ollama (air-gap)
      v shell-out (Docker)            v HTTP
INGESTION ADAPTERS          EXTERNAL: ADO REST API, GitHub API, GEI
  gh actions-importer (Docker)
  SourceAdapter trait (ADO -> ...)
```

## The conversion loop

Per pipeline:

1. Run the Importer `dry-run`.
2. Parse the log for unsupported steps, partial constructs, and manual tasks into
   typed **Gap** records.
3. Build a grounded LLM request per gap — the source snippet, the Importer's
   converted output, the specific failure from the log, and repo context.
4. Assemble the augmented workflow plus rationale and risk, and persist it as a
   **Proposal** awaiting review.

## Proposal lifecycle

```
draft -> in_review -> approved / changes_requested -> committed -> validated
```

Illegal transitions are rejected, and every transition is appended to the audit log.

## Two seams that stay abstract

These two traits are the extension points; orchestration code depends only on them.

### SourceAdapter

`discover` / `enumerate_pipelines` / `fetch_definition` / `fetch_service_connections`
/ `fetch_variable_groups` / `task_inventory`.

Azure DevOps is the first implementation; Jenkins, GitLab, CircleCI, Travis, and
Bamboo follow the same contract, with Bitbucket as a discovery-only source. A single
conformance suite runs the trait contract against all of them from captured fixtures.
Classic (designer) pipelines are classified separately from YAML pipelines.

### LlmProvider

Returns structured JSON:

```json
{ "proposed_yaml": "...", "rationale": "...", "risk_flags": [], "verify_steps": [], "confidence": 0.0 }
```

Orchestration calls only this trait, never a vendor SDK directly. Implementations
cover Anthropic, Google Gemini, GitHub Models/Copilot, Azure OpenAI, Google Vertex,
an OpenAI-compatible endpoint (including Bedrock gateways), and local Ollama.

Routing policy: bulk and cheap work to a local or small model; hard reasoning and
documentation to a frontier model. Air-gap mode forces everything local.

## Deterministic risk

The risk score is computed from weighted, explainable factors — container jobs,
variable groups, service connections, multi-stage gates — and lands in one of three
bands (Green, Amber, Red). The model never produces the score; it only explains and
flags. This keeps the risk assessment reproducible and defensible.

## Persistence and tenancy

The control plane runs over Postgres (server / multi-tenant) or SQLite (local /
air-gap) with the same schema. Secret *names* discovered during audit are data;
secret *values* are never fetched or stored — variable groups and service
connections record names and types only.
