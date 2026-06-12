# bifrost-mcp

A [Model Context Protocol](https://modelcontextprotocol.io) server that exposes Bifrost's
**read-only** migration context to an agent — the Copilot coding agent, Claude, or any
MCP-capable IDE — so it can resolve conversion gaps grounded in the real portfolio rather than
guessing.

It is read-only by design: it only surfaces context. Every mutation (convert, approve, commit)
stays behind the control-plane API's human-approval and audit-logged path. The server proxies the
control plane, so it inherits the API's auth and **air-gap** posture.

## Tools

| Tool | What it returns |
|------|-----------------|
| `bifrost_portfolio` | Every pipeline: project, classification, risk band + score, converted ratio, status |
| `bifrost_forecast` | Deterministic GitHub Actions cost + capacity for the target org |
| `bifrost_coverage` | Completeness matrix — every ADO moving part mapped to its GitHub equivalent + status |
| `bifrost_assessment` | Source assessment — pipeline mix, risk, inventory density |
| `bifrost_readiness` | Target GitHub pre-flight readiness checklist |
| `bifrost_report` | Pre-migration status report (Markdown), optionally `project`-scoped |
| `validate_workflow` | Basic structural check of a workflow YAML (`on:` / `jobs:`) |

## Run

The server speaks newline-delimited JSON-RPC 2.0 over stdio. It proxies the control plane at
`BIFROST_API_URL` (default `http://127.0.0.1:8080`).

```bash
cargo build -p bifrost-mcp
BIFROST_API_URL=http://127.0.0.1:8080 ./target/debug/bifrost-mcp
```

## Register with an MCP client

Most clients take a JSON config naming the command. For example:

```json
{
  "mcpServers": {
    "bifrost": {
      "command": "/path/to/bifrost-mcp",
      "env": { "BIFROST_API_URL": "http://127.0.0.1:8080" }
    }
  }
}
```

Point an agent at it and ask it to convert a pipeline; it can pull the portfolio, the gaps for a
pipeline (via the report), the coverage and readiness, and validate its proposed workflow — all
grounded, all read-only.
