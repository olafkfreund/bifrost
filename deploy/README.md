# Deploying Bifrost

Two ways to run Bifrost: **Docker Compose** for a single self-hosted box, and a
**Helm chart** for Kubernetes (EKS / AKS / GKE). Both run the same two
components — the control-plane API and the review portal (nginx, which serves the
SPA and proxies `/api` to the API).

## Docker Compose (self-host v1)

SQLite-backed, air-gap by default — nothing leaves the box.

```bash
cd deploy
docker compose up --build
# portal + API on http://localhost:8080
```

Configuration (environment, or a `deploy/.env` file):

| Variable | Default | Purpose |
|---|---|---|
| `BIFROST_DB` | `sqlite:/data/bifrost.db` | datastore; set `postgres://…` for server mode |
| `BIFROST_AIR_GAP` | `1` | disable frontier LLM providers; keep data local |
| `BIFROST_SIGNING_KEY` | _(dev key)_ | HMAC key for attestation export (#62/#63) — **set in production** |
| `BIFROST_SIGNING_KEY_ID` | `bifrost-dev` | key id stamped into exported attestations |
| `BIFROST_PORTAL_PORT` | `8080` | host port for the portal |

Server mode (bundled Postgres):

```bash
BIFROST_DB=postgres://bifrost:bifrost@postgres:5432/bifrost \
  docker compose --profile postgres up --build
```

## Helm (Kubernetes)

```bash
helm install bifrost deploy/helm/bifrost \
  --set image.api.repository=ghcr.io/olafkfreund/bifrost-api \
  --set image.portal.repository=ghcr.io/olafkfreund/bifrost-portal \
  --set signingKey.value="$(openssl rand -hex 32)"
```

Defaults: one API replica with a 1Gi SQLite PVC, one portal replica, air-gap on,
no ingress. Key values:

| Value | Default | Purpose |
|---|---|---|
| `api.db` | `sqlite:/data/bifrost.db` | `postgres://…` switches to server mode and drops the PVC |
| `api.airGap` | `true` | air-gap mode |
| `api.persistence.{enabled,size}` | `true`, `1Gi` | SQLite volume (ignored for Postgres) |
| `signingKey.value` / `signingKey.existingSecret` | — | attestation signing key (chart Secret, or one you manage) |
| `ingress.enabled` / `ingress.host` | `false` | expose the portal (it fronts `/api` too) |

Render or lint before applying:

```bash
helm lint deploy/helm/bifrost
helm template bifrost deploy/helm/bifrost --set ingress.enabled=true,ingress.host=bifrost.example.com
```

No ingress? Port-forward the portal:

```bash
kubectl port-forward svc/bifrost-portal 8080:80
# http://localhost:8080
```

## Images

### Pull a published image (ready to run)

Tagged releases publish signed images to GHCR (built by `.github/workflows/release.yml`):

```bash
docker pull ghcr.io/olafkfreund/bifrost-api:latest
docker pull ghcr.io/olafkfreund/bifrost-portal:latest
```

Point the Helm chart at them:

```bash
helm install bifrost deploy/helm/bifrost \
  --set image.api.repository=ghcr.io/olafkfreund/bifrost-api \
  --set image.portal.repository=ghcr.io/olafkfreund/bifrost-portal \
  --set signingKey.value="$(openssl rand -hex 32)"
```

### Verify the signature and SBOM (supply-chain)

Every published image is **signed with cosign (keyless / Sigstore)** and ships an
**SPDX SBOM attestation** — no long-lived keys. Verify before you run, in an enterprise:

```bash
# 1. Verify the image was built by this repo's release workflow.
cosign verify ghcr.io/olafkfreund/bifrost-api:latest \
  --certificate-identity-regexp '^https://github.com/olafkfreund/bifrost/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com

# 2. Verify + print the SBOM attestation (what's inside the image).
cosign verify-attestation ghcr.io/olafkfreund/bifrost-api:latest \
  --type spdxjson \
  --certificate-identity-regexp '^https://github.com/olafkfreund/bifrost/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  | jq -r '.payload | @base64d | fromjson | .predicate' > bifrost-api.sbom.spdx.json
```

The SBOM is also attached to each workflow run as a build artifact.

### Build locally (for testing / development)

The Dockerfiles live in `deploy/docker/` and build from the **repo root**:

```bash
docker build -f deploy/docker/api.Dockerfile    -t bifrost-api    .
docker build -f deploy/docker/portal.Dockerfile -t bifrost-portal .
```

The API is a static Rust binary on `debian:bookworm-slim` (non-root); the portal
is the Vite build (`VITE_API=http`) served by nginx. In Kubernetes the chart
mounts a templated nginx config so `/api` targets the in-cluster API service.
