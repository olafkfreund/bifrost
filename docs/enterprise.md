---
title: Enterprise golden path
layout: default
nav_order: 6
permalink: /enterprise
description: "Deploy, gate, secure and operate Bifrost in an enterprise — the golden path from a pulled, signed image to SSO, RBAC, multi-tenancy and air-gap, with every control opt-in."
---

# Enterprise golden path
{: .fs-9 }

From a verified image to a hardened, single-sign-on, multi-tenant deployment — in the order
you'd actually do it, with every production control opt-in and off by default.
{: .fs-6 .fw-300 }

---

## The shape of a Bifrost deployment

Two containers — the **control-plane API** (Rust) and the **review portal** (nginx serving the
SPA and proxying `/api`). One datastore — **SQLite** for a single box, **Postgres** for the
multi-tenant server, same schema over both. Everything else (a model, GitHub, Azure DevOps) is
an outbound edge that you turn on deliberately.

The guiding principle: **out of the box Bifrost runs open, air-gapped and inert** — mock
providers, no auth, no external calls. You reach production by *enabling* controls, never by
disabling safety.

---

## Step 1 — Deploy

Pick the substrate. Both are covered in detail in [`deploy/`](https://github.com/olafkfreund/bifrost/tree/main/deploy).

- **Single box (Docker Compose)** — SQLite, air-gap by default:

  ```bash
  cd deploy && docker compose up --build      # portal + API on http://localhost:8080
  ```

- **Kubernetes (Helm)** — EKS / AKS / GKE:

  ```bash
  helm install bifrost deploy/helm/bifrost \
    --set image.api.repository=ghcr.io/olafkfreund/bifrost-api \
    --set image.portal.repository=ghcr.io/olafkfreund/bifrost-portal \
    --set signingKey.value="$(openssl rand -hex 32)"
  ```

## Step 2 — Pull a verified image

Tagged releases publish **cosign-signed** images with **SPDX SBOM attestations** to GHCR. In a
regulated enterprise, verify provenance before you run:

```bash
cosign verify ghcr.io/olafkfreund/bifrost-api:latest \
  --certificate-identity-regexp '^https://github.com/olafkfreund/bifrost/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

The full pull + `verify-attestation` + SBOM-extraction recipe is in
[`deploy/` → Images](https://github.com/olafkfreund/bifrost/tree/main/deploy#images).

## Step 3 — Choose your gating posture

Every live, paid or outward-facing path is a separate opt-in environment flag. Set only what
the posture requires.

| Control | Flag | Default | Effect when set |
|---------|------|---------|-----------------|
| Datastore | `BIFROST_DB` | `sqlite:/data/bifrost.db` | `postgres://…` switches to multi-tenant server mode |
| Air-gap | `BIFROST_AIR_GAP` | `1` (compose) | Forces every model call local; disables frontier providers. `BIFROST_AIR_GAP_LOCK` makes it non-toggleable at runtime |
| Authentication | `BIFROST_AUTH` | unset (open) | `entra` / `oidc` / `github` require a valid bearer on `/api/*` |
| Live conversion | `BIFROST_CONVERT_LIVE` | unset (mock) | Uses the real Importer Docker + configured LLM, not the mock |
| Live commit | `BIFROST_COMMIT_LIVE` | unset (mock PR) | Opens a real GitHub pull request on commit |
| Live validation | `BIFROST_VALIDATE_LIVE` | unset (mock) | Triggers a real sandbox `workflow_dispatch` run |
| Editor commit | `BIFROST_MCP_COMMIT` | unset (off) | Allows the MCP `bifrost_commit` tool to open PRs |
| Attestation signing | `BIFROST_SIGNING_KEY` | dev key | **Set in production** — HMAC key for signed attestation export |
| Target repo | `BIFROST_GH_REPO` / `BIFROST_GH_BASE` | `example/sandbox` / `main` | Where converted workflows are committed |

Three reference postures:

- **Air-gapped pilot** — `BIFROST_AIR_GAP=1`, everything else default. No external calls; mock
  commit/validate. Prove the workflow on a disconnected box.
- **Connected enterprise** — add `BIFROST_AUTH=entra`, `BIFROST_CONVERT_LIVE=1`, a real
  `BIFROST_SIGNING_KEY`, Postgres, and a GitHub App for repo writes. Leave commit/validate
  mock until teams trust the proposals.
- **Full production** — additionally `BIFROST_COMMIT_LIVE=1` + `BIFROST_VALIDATE_LIVE=1`, with
  RBAC and tenancy enforced (below).

## Step 4 — Single sign-on and RBAC

Authentication is **opt-in**. Unset, the API runs open and every request is the local admin
(logged loudly at startup). Set `BIFROST_AUTH` to pick a provider; the API then validates the
bearer token on every `/api/*` request (except `/api/health`) and maps it to a Bifrost identity.
All providers plug into the same `Authenticator` seam, so the RBAC and tenancy below are
provider-agnostic. If a configured provider can't be initialised, the API logs an error and
falls back to open mode rather than failing closed.

| `BIFROST_AUTH` | Provider | How the bearer is validated |
|----------------|----------|-----------------------------|
| `entra` | Microsoft Entra ID (Azure AD) | OIDC JWT: signature against Entra's JWKS + issuer / audience / expiry |
| `oidc` | Any OIDC issuer (Keycloak, Auth0, Okta, Ping, …) | OIDC JWT: signature against the issuer's JWKS + issuer / audience / expiry |
| `github` | Sign in with GitHub | GitHub user API call (`GET /user`) — GitHub OAuth issues opaque tokens, not OIDC ID tokens |

### Entra ID

```bash
BIFROST_AUTH=entra
BIFROST_ENTRA_TENANT_ID=<your-tenant-guid>
BIFROST_ENTRA_AUDIENCE=<api-app-client-id>
# Issuer + JWKS URI are derived from the tenant; override only for sovereign clouds:
# BIFROST_ENTRA_ISSUER=...   BIFROST_ENTRA_JWKS_URI=...
```

The portal signs the user in with the standard OIDC authorization-code + PKCE flow (MSAL) and
sends the resulting bearer to the API. JWKS is cached and refreshed hourly (Entra rotates keys).

### Generic OIDC (Keycloak, Auth0, Okta, …)

For any standards-compliant OIDC issuer, give the issuer, JWKS URI and audience explicitly. This
makes **Keycloak** work directly — including Keycloak **brokering** to Entra (Keycloak federates
to Entra upstream and issues its own tokens to Bifrost, so Bifrost only ever trusts Keycloak).

```bash
BIFROST_AUTH=oidc
BIFROST_OIDC_ISSUER=https://keycloak.example.com/realms/bifrost
BIFROST_OIDC_JWKS_URI=https://keycloak.example.com/realms/bifrost/protocol/openid-connect/certs
BIFROST_OIDC_AUDIENCE=bifrost-api
# Role claims are read from `roles` and/or `groups`. Tenant defaults to the `tid`
# claim; point it at your provider's claim if different (Keycloak realm, org id, …):
# BIFROST_OIDC_TENANT_CLAIM=org_id
```

JWKS is cached and refreshed hourly, exactly as for Entra. Role claims map through the same table
below; map your realm/client roles (or groups) to `viewer` / `reviewer` / `admin` in the IdP.

### Sign in with GitHub

GitHub's OAuth web flow issues opaque access tokens (not verifiable OIDC ID tokens), so Bifrost
validates the bearer by calling the GitHub user API with it. Everyone defaults to **Viewer**;
list the GitHub logins that should be **Admin** explicitly.

```bash
BIFROST_AUTH=github
# Logins granted Admin (comma-separated, case-insensitive); everyone else is Viewer:
BIFROST_GITHUB_ADMIN_LOGINS=octocat,ada-lovelace
# For GitHub Enterprise Server, point at your instance's API base:
# GITHUB_API_BASE=https://github.example.com/api/v3
```

### Roles

Bifrost has three roles, mapped from token claims (`roles` / `groups`). Each API route requires
a minimum role; reads need `Viewer`, lifecycle actions need `Reviewer`, sensitive
config/compliance needs `Admin`.

| Role | Can | Maps from claims |
|------|-----|------------------|
| **Viewer** | Read everything: portfolio, forecast, coverage, readiness, reports, proposals | `viewer`, `reader`, `read` |
| **Reviewer** | Viewer + convert, edit, transition, commit proposals | `reviewer`, `approver`, `editor` |
| **Admin** | Reviewer + connections, routing, audit-pack export, air-gap toggle | `admin`, `administrator`, `owner` |

### Multi-tenancy

In Postgres/server mode, proposals are owned by the caller's tenant; cross-tenant reads return
`404` (existence never leaks across tenants). One Bifrost instance can serve many isolated teams.

### Repo writes — a least-privilege GitHub App

Committing a converted workflow opens a pull request via a GitHub App scoped to the target
repositories (contents + pull-requests), not a personal token. Service connections are federated
to GitHub via **OIDC** rather than copying secrets — Bifrost records secret *names*, never values.

{: .note }
> **Identity providers.** Bifrost ships three built-in SSO providers behind one `Authenticator`
> seam: **Entra ID** (`BIFROST_AUTH=entra`), a **generic-OIDC** authenticator
> (`BIFROST_AUTH=oidc`, so **Keycloak** works directly — including Keycloak brokering to Entra),
> and a **GitHub login** (`BIFROST_AUTH=github`)
> ([#286](https://github.com/olafkfreund/bifrost/issues/286)). The RBAC + tenancy above are
> provider-agnostic and apply unchanged across all three.

## Step 5 — Air-gap

For disconnected or data-residency-bound environments: set `BIFROST_AIR_GAP=1` (and
`BIFROST_AIR_GAP_LOCK=1` to make it non-toggleable). The model router then forces every LLM call
to a local provider (Ollama / llama.cpp); frontier providers are disabled by config, and no
pipeline definition leaves the box. Point Bifrost at your local model and run conversions with
zero egress.

---

## Hardening checklist

- [ ] Pulled images **verified** with `cosign verify` (provenance + SBOM attestation).
- [ ] `BIFROST_SIGNING_KEY` set to a real secret (not the dev key) and stored in your secrets manager.
- [ ] `BIFROST_AUTH` set to your provider (`entra` / `oidc` / `github`) and configured; portal wired to your IdP.
- [ ] Roles assigned in the IdP (Viewer / Reviewer / Admin) — least privilege per team.
- [ ] Postgres for server mode; per-tenant isolation verified.
- [ ] GitHub App installed, scoped to the target repos only; service connections federated via OIDC.
- [ ] Live flags (`CONVERT` / `COMMIT` / `VALIDATE` / `MCP_COMMIT`) enabled **only** where intended.
- [ ] Air-gap (`BIFROST_AIR_GAP=1` `+_LOCK`) for disconnected / residency-bound installs.
- [ ] Attestation packs exported and retained for audit.

---

Where to go next: [deploy details](https://github.com/olafkfreund/bifrost/tree/main/deploy) ·
the [architecture](/architecture) (trust model + design rationale) · the [editor guide](/mcp)
(MCP from VS Code and other IDEs).
