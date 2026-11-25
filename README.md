# omniroute-rs

> **Language:** **English** | [Bahasa Indonesia](README.id.md)

A native **Rust** port of the [OmniRoute](https://github.com/cheaper-inference/OmniRoute) AI Gateway, synced with upstream **`release/v3.8.55`** — shipped as a **single native binary (~10–11 MB)** with an ultra-low memory footprint (~15–25 MB RSS), zero Node.js, zero Electron, and zero external frontend assets. Built to run effortlessly on small VPS instances, mini PCs, Raspberry Pi, and ARM64 set-top boxes (such as Armbian HG680P with <1 GB RAM).

---

## Key Features

- **394 Providers & 3,239 Models** (including **82 free-tier providers**) embedded directly into the binary (`v3.8.55`).
- **5 Inference Wire Surfaces**: OpenAI Chat Completions (`/v1/chat/completions`), Anthropic Messages (`/v1/messages`), OpenAI Responses (`/v1/responses`), Native Gemini (`/v1beta/models/*`), and Legacy Completions (`/v1/completions`).
- **Extended Multimodal & Utility APIs**: `/v1/embeddings`, `/v1/images/*`, `/v1/audio/*`, `/v1/rerank`, `/v1/moderations`, `/v1/decisions` (`v3.8.55`), `/v1/search` (8 search engines), `/v1/web/fetch` (SSRF-guarded), and `/v1/me/*`.
- **22 Routing Strategies & Resilience**: `priority`, `round-robin`, `weighted`, `random`, `least-used`, `cost-optimized`, `strict-random`, `p2c`, `lkgp`, `jev`, `expiry-first`, `fill-first`, `auto` (11 sub-strategies), 4-state circuit breakers, model lockouts, and `Retry-After` parsing.
- **OAuth 2.0 (17 Providers) & 7 Priority Provider Executors**:
  - PKCE `S256` & RFC 8628 Device-Code flows + background token refresh scheduler (`BG_REFRESH_GOOGLE_DELAY_MS`, `TOKEN_REFRESH_RETRY_JITTER=1`, `HEALTHCHECK_PER_PROVIDER_CONCURRENCY`).
  - Dedicated executors: **Claude / Claude Code** (gzip request body `>= 4096B` + automatic uncompressed retry & 1h latch on `415`, conditional `skills-2025-10-02`), **Codex / ChatGPT**, **GitHub Copilot / GHE** (auto-mint token, Claude → `/v1/messages`, Codex → `/responses`), **Antigravity & Gemini CLI** (`v1internal` envelope + multi-endpoint fallback), **AWS Bedrock** (pure-Rust **AWS SigV4** + Converse API), **GCP Vertex AI** (Express & project-scoped), and **Azure OpenAI / AI Foundry**.
- **Token & Cost Optimization**: **RTK** command-aware compression (error-preserving `#21081`) + **Caveman** prompt compression (EN & ID) + **Adaptive Context Budget** + **Dual-Layer Semantic Cache** (exact SHA-256 + sparse TF cosine similarity, isolated per API key).
- **Security & Gamification**: Prompt-injection guardrails, PII masking + reversible `PiiVault`, `agent-guard` (`off`/`monitor`/`block`), plus XP, levels, daily streaks, and badges.
- **Live TCP/TLS HTTP CONNECT MITM Proxy**: Pure-Rust ECDSA P-256 Root CA + dynamic per-host leaf certificates (`rustls`, zero OpenSSL dependency) targeting 10 AI coding agents (Claude Code, Codex, Copilot, Antigravity, Cursor, Kiro, Zed, OpenCode, Trae, GHE Copilot).
- **A2A Protocol v0.3 (+ v1.0 Compat)**, **Persistent Agent Memory** (SQLite WAL + TF-cosine retrieval), **Skills Registry**, and **24 MCP Tools** (stdio & HTTP).
- **Embedded 8-Tab Web Dashboard SPA** at `/` & `/dashboard` + **Direct Importer for upstream Node.js `storage.sqlite` / `db.json`**.

---

## Table of Contents

1. [Prerequisites & Building from Source](#1-prerequisites--building-from-source)
2. [Cross-Compiling for Linux ARM64 / STB (HG680P / Armbian / Raspberry Pi)](#2-cross-compiling-for-linux-arm64--stb-hg680p--armbian--raspberry-pi)
3. [Production Deployment with `systemd` (Linux / STB)](#3-production-deployment-with-systemd-linux--stb)
4. [Running the Gateway & Opening the Web Dashboard](#4-running-the-gateway--opening-the-web-dashboard)
5. [Client Configuration (OpenAI SDK, Claude Code, Cursor, Cline, OpenCode)](#5-client-configuration-openai-sdk-claude-code-cursor-cline-opencode)
6. [Configuring OAuth 2.0, Cloud Providers (Bedrock, Vertex, Azure) & Combos](#6-configuring-oauth-20-cloud-providers-bedrock-vertex-azure--combos)
7. [Enabling the MITM TLS Proxy & Trusting the Root CA](#7-enabling-the-mitm-tls-proxy--trusting-the-root-ca)
8. [Migrating Data from Node.js OmniRoute (`storage.sqlite` / `db.json`)](#8-migrating-data-from-nodejs-omniroute-storagesqlite--dbjson)
9. [MCP Server (24 Tools) & A2A v0.3 / v1.0 Integration](#9-mcp-server-24-tools--a2a-v03--v10-integration)
10. [Complete CLI Reference](#10-complete-cli-reference)
11. [Environment Variables Reference](#11-environment-variables-reference)
12. [API Endpoints Reference](#12-api-endpoints-reference)
13. [Syncing the Provider Catalog from Upstream OmniRoute](#13-syncing-the-provider-catalog-from-upstream-omniroute)
14. [Workspace Architecture (14 Crates)](#14-workspace-architecture-14-crates)
15. [Troubleshooting](#15-troubleshooting)

---

## 1. Prerequisites & Building from Source

### Prerequisites
- **Rust toolchain** `>= 1.80` (`rustup` + `cargo`)
- **C compiler** (needed for bundled SQLite in `rusqlite` and `ring`):
  - **Linux (Ubuntu/Debian/Armbian)**: `sudo apt update && sudo apt install -y build-essential git`
  - **Windows**: MinGW-w64 (`g++` / target `x86_64-pc-windows-gnu`, preconfigured in `.cargo/config.toml`) or Visual Studio Build Tools (`msvc`).
  - **macOS**: `xcode-select --install`

### Clone & Build

#### Linux / macOS
```bash
git clone https://github.com/ngnawfal/omniroute-rs.git
cd omniroute-rs

# Run the full test suite (102 tests across 14 crates)
cargo test --workspace

# Build native release binary (explicit host target overrides Windows default in .cargo/config.toml)
cargo build --release --target "$(rustc -vV | awk '/host:/ {print $2}')" -p omni-server

# Resulting binary:
ls -lh target/*/release/omniroute-rs
```

#### Windows (PowerShell)
```powershell
git clone https://github.com/ngnawfal/omniroute-rs.git
cd omniroute-rs

# Test & build release
cargo test --workspace
cargo build --release -p omni-server

# Verify binary version & embedded catalog
.\target\x86_64-pc-windows-gnu\release\omniroute-rs.exe version
```

---

## 2. Cross-Compiling for Linux ARM64 / STB (HG680P / Armbian / Raspberry Pi)

Low-power ARM64 devices like an **HG680P STB** (~787 MB RAM, small eMMC) run `omniroute-rs` effortlessly (~15–25 MB RSS), but compiling Rust directly on the device is slow and disk-heavy. The fastest approach is cross-compiling on an `x86_64` workstation using **Docker**, then uploading the single ~10 MB ELF binary.

### Fast Docker Cross-Compile (`x86_64` → `aarch64-unknown-linux-gnu`)
```bash
docker run --rm --platform linux/amd64 \
  -v "$(pwd):/workspace:ro" \
  -v "$(pwd)/dist:/out" \
  rust:1-bookworm bash -c '
    set -euo pipefail
    apt-get update -qq && apt-get install -y -qq --no-install-recommends gcc-aarch64-linux-gnu libc6-dev-arm64-cross >/dev/null
    rustup target add aarch64-unknown-linux-gnu
    mkdir -p /build/src && cp -a /workspace/Cargo.toml /workspace/Cargo.lock /workspace/crates /build/src/
    cd /build/src
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
    export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
    export AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar
    cargo build --release --target aarch64-unknown-linux-gnu -p omni-server
    cp target/aarch64-unknown-linux-gnu/release/omniroute-rs /out/omniroute-rs-aarch64
  '
```

Upload the binary to your server/STB via `scp`:
```bash
scp dist/omniroute-rs-aarch64 root@192.168.11.209:/opt/omniroute-rs/omniroute-rs
```

---

## 3. Production Deployment with `systemd` (Linux / STB)

On the target Linux/ARM64 server (`root`):

```bash
mkdir -p /opt/omniroute-rs/data
chmod 755 /opt/omniroute-rs/omniroute-rs
ln -sf /opt/omniroute-rs/omniroute-rs /usr/local/bin/omniroute-rs

# Environment configuration
cat > /opt/omniroute-rs/omniroute-rs.env <<'EOF'
OMNIROUTE_PORT=20128
OMNIROUTE_DATA_DIR=/opt/omniroute-rs/data
RUST_LOG=info
# Optional: automatically start MITM proxy alongside the gateway
# OMNIROUTE_MITM_PORT=20443
EOF

# systemd unit file
cat > /etc/systemd/system/omniroute-rs.service <<'EOF'
[Unit]
Description=omniroute-rs Native Rust AI Gateway (OmniRoute v3.8.55)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=/opt/omniroute-rs
EnvironmentFile=-/opt/omniroute-rs/omniroute-rs.env
ExecStart=/opt/omniroute-rs/omniroute-rs serve
Restart=always
RestartSec=3
LimitNOFILE=65535
MemoryHigh=256M
MemoryMax=512M

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now omniroute-rs.service
systemctl status omniroute-rs.service --no-pager
```

### Day-2 Operations
```bash
# Follow live logs
journalctl -u omniroute-rs -f

# Restart after editing .env
systemctl restart omniroute-rs

# Run diagnostics & health check
omniroute-rs doctor
curl -s http://127.0.0.1:20128/api/health
```

---

## 4. Running the Gateway & Opening the Web Dashboard

### Manual Startup
```bash
# Default port 20128
omniroute-rs serve

# Custom port + automatic MITM proxy on port 20443
omniroute-rs serve --port 20128 --mitm-port 20443
```

### Embedded Web Dashboard
Open in any browser:
- Local: **`http://127.0.0.1:20128/dashboard`** (or `http://127.0.0.1:20128/`)
- LAN / Remote: **`http://<SERVER-IP>:20128/dashboard`** (e.g., `http://192.168.11.209:20128/dashboard`)

> **Authentication Posture:**
> - **Default (0 API keys in DB):** local open-door mode for initial setup.
> - **Once 1 or more API keys are created** (`omniroute-rs keys create` or via the *Keys & Guardrails* tab): strict auth is enforced across `/v1/*`, `/a2a`, and mutating `/api/*` endpoints via `Authorization: Bearer <API_KEY>` or `x-api-key: <API_KEY>`. Paste your key in the top-right *Gateway API Key* box in the Dashboard and click **Save Key**.

---

## 5. Client Configuration (OpenAI SDK, Claude Code, Cursor, Cline, OpenCode)

### OpenAI SDK (Python / Node.js / curl)
```bash
curl http://127.0.0.1:20128/v1/chat/completions \
  -H "Authorization: Bearer sk-omni-YOUR_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "openai/gpt-6.1-sol",
    "messages": [{"role": "user", "content": "Hello from omniroute-rs!"}]
  }'
```

### Anthropic SDK / Claude Code
```bash
export ANTHROPIC_BASE_URL="http://127.0.0.1:20128"
export ANTHROPIC_API_KEY="sk-omni-YOUR_KEY"
claude
```
The gateway transparently translates `/v1/messages` (both JSON and SSE streaming, including `thinking` and `tool_use`) to any target provider or combo.

### OpenAI Responses API & Native Gemini
- **Responses API**: `POST http://127.0.0.1:20128/v1/responses`
- **Native Gemini**: `POST http://127.0.0.1:20128/v1beta/models/gemini/gemini-2.5-pro:generateContent`

### Cursor / Cline / Roo Code / OpenCode / Continue
- **Base URL**: `http://<SERVER-IP>:20128/v1`
- **API Key**: generated via `omniroute-rs keys create` (or any placeholder if open-door mode).
- **Model**: `provider/model` (e.g. `claude/claude-opus-4-7`, `github/claude-sonnet-4.5`, `groq/openai/gpt-oss-120b`, or any custom **Combo** name).

---

## 6. Configuring OAuth 2.0, Cloud Providers (Bedrock, Vertex, Azure) & Combos

### Add an API-Key Credential
Via the Dashboard (*Providers & OAuth* tab) or REST API:
```bash
curl -X POST http://127.0.0.1:20128/api/providers/credentials \
  -H "Content-Type: application/json" \
  -d '{
    "provider": "anthropic",
    "connection_id": "main",
    "auth_type": "x-api-key",
    "api_key": "sk-ant-...",
    "base_urls": [],
    "models": [],
    "label": "Anthropic Main"
  }'
```

### OAuth 2.0 Login (Claude Code, Codex, ChatGPT, GitHub Copilot, Antigravity, Gemini CLI, etc.)
```bash
# 1. List all 17 supported OAuth providers
curl http://127.0.0.1:20128/api/oauth/providers

# 2. Start PKCE flow (e.g. codex) -> open authorize_url in browser
curl -X POST http://127.0.0.1:20128/api/oauth/codex/authorize -H "Content-Type: application/json" -d '{}'

# 3. Exchange authorization code + state
curl -X POST http://127.0.0.1:20128/api/oauth/codex/callback \
  -H "Content-Type: application/json" \
  -d '{"code": "CALLBACK_CODE", "state": "STATE_FROM_STEP_2"}'

# Or GitHub Copilot (RFC 8628 Device-Code flow):
curl -X POST http://127.0.0.1:20128/api/oauth/github/authorize -H "Content-Type: application/json" -d '{}'
# Enter user_code at verification_uri, then poll:
curl -X POST http://127.0.0.1:20128/api/oauth/github/poll \
  -H "Content-Type: application/json" \
  -d '{"state": "STATE_FROM_AUTHORIZE"}'
```

### AWS Bedrock (IAM SigV4 or Bedrock Native Bearer Key)
```bash
curl -X POST http://127.0.0.1:20128/api/providers/credentials \
  -H "Content-Type: application/json" \
  -d '{
    "provider": "bedrock",
    "connection_id": "aws-us-east-1",
    "auth_type": "apikey",
    "api_key": "",
    "base_urls": [],
    "models": [],
    "provider_specific_json": {
      "region": "us-east-1",
      "aws_access_key_id": "AKIA...",
      "aws_secret_access_key": "..."
    }
  }'
```

### GCP Vertex AI & Azure OpenAI / AI Foundry
- **Vertex AI**: set `provider_specific_json: {"project_id": "my-gcp-proj", "region": "us-central1"}` (OAuth Bearer) or supply a Gemini Express API key in `api_key`.
- **Azure**: set `provider: "azure"`, `auth_type: "api-key"`, `base_urls: ["https://<resource>.openai.azure.com"]` (Classic) or `["https://<project>.services.ai.azure.com"]` (Foundry auto-detected).

### Create a Failover Combo (22 Strategies)
```bash
curl -X POST http://127.0.0.1:20128/api/combos \
  -H "Content-Type: application/json" \
  -d '{
    "name": "coding-fast",
    "strategy": "jev",
    "models": [
      {"provider": "claude", "model": "claude-opus-4-7", "weight": 2.0},
      {"provider": "github", "model": "claude-sonnet-4.5", "weight": 1.5},
      {"provider": "groq", "model": "openai/gpt-oss-120b", "weight": 1.0, "fallback_only": true}
    ]
  }'
```

---

## 7. Enabling the MITM TLS Proxy & Trusting the Root CA

`omniroute-rs` includes a native **HTTP CONNECT + TLS Interception MITM** proxy (`rustls` + ECDSA P-256 Root CA auto-generated in `<data_dir>/mitm/ca.crt`).

### Starting the Proxy
- **Dashboard**: open *MITM & A2A* tab → click **Start Proxy** (default port `20443`).
- **Standalone CLI**:
  ```bash
  omniroute-rs mitm --port 20443 --gateway http://127.0.0.1:20128
  ```
- **Alongside `serve`**: `omniroute-rs serve --port 20128 --mitm-port 20443`.

### Trusting the Root CA on Clients
Download the CA certificate from `http://<SERVER-IP>:20128/api/mitm/ca.crt`:
- **Node.js / Claude Code / Codex / OpenCode**:
  ```bash
  export HTTPS_PROXY="http://127.0.0.1:20443"
  export NODE_EXTRA_CA_CERTS="/path/to/omniroute-mitm-ca.crt"
  ```
- **Linux (system-wide)**:
  ```bash
  sudo cp omniroute-mitm-ca.crt /usr/local/share/ca-certificates/omniroute-mitm.crt
  sudo update-ca-certificates
  ```
- **Windows**: double-click `omniroute-mitm-ca.crt` → *Install Certificate* → *Trusted Root Certification Authorities*.

---

## 8. Migrating Data from Node.js OmniRoute (`storage.sqlite` / `db.json`)

Migrate accounts, OAuth tokens, combos, API keys, settings, model aliases, and usage history from an existing Node.js OmniRoute installation non-destructively (the source DB is opened read-only):

### Via CLI
```bash
# Direct import from upstream storage.sqlite
omniroute-rs db import /root/.omniroute/storage.sqlite

# Or from legacy db.json / exported JSON snapshot
omniroute-rs db import /path/to/db.json

# Verify SQLite WAL integrity & row counts across all 15 tables
omniroute-rs db status

# Create an atomic online backup (VACUUM INTO)
omniroute-rs db backup /opt/omniroute-rs/data/backups/manual.db
```

### Via Web Dashboard / REST API
Open the **Database & Migration** tab in the Dashboard or call `POST /api/db/import/upstream-sqlite` with `{"path": "/path/to/storage.sqlite"}`.

---

## 9. MCP Server (24 Tools) & A2A v0.3 / v1.0 Integration

### MCP over stdio (Claude Desktop / Cursor / Windsurf / Cline)
```json
{
  "mcpServers": {
    "omniroute-rs": {
      "command": "/usr/local/bin/omniroute-rs",
      "args": ["mcp"],
      "env": {
        "OMNIROUTE_DATA_DIR": "/opt/omniroute-rs/data"
      }
    }
  }
}
```
Also available over HTTP at `POST http://<SERVER-IP>:20128/api/mcp` (JSON-RPC 2.0).

### A2A Protocol v0.3 (+ v1.0 Compatibility)
- **Agent Card**: `GET http://<SERVER-IP>:20128/.well-known/agent.json`
- **JSON-RPC 2.0**: `POST http://<SERVER-IP>:20128/a2a` supporting `message/send`, `message/stream` (SSE), `tasks/get`, `tasks/cancel`, and v1.0 method aliases (`SendMessage`, `SendStreamingMessage`).

---

## 10. Complete CLI Reference

```bash
omniroute-rs serve [--port N] [--mitm-port N]          # Start HTTP gateway (+ optional MITM proxy)
omniroute-rs mcp                                       # Run JSON-RPC 2.0 MCP server over stdio (24 tools)
omniroute-rs mitm [--port N] [--gateway URL]           # Run standalone TLS/CONNECT MITM proxy
omniroute-rs status                                    # JSON summary of catalog, DB, OAuth & MITM
omniroute-rs doctor                                    # Environment, SQLite, CA cert & port diagnostics
omniroute-rs providers [--free] [--q TEXT] [--limit N] # Search/list embedded 394 providers
omniroute-rs models [--provider ID] [--q TEXT]         # Search/list embedded 3,239 models
omniroute-rs keys list | create [--label L] [--rpm N] | delete <KEY>
omniroute-rs combos list | simulate <MODEL> [--strategy S]
omniroute-rs oauth list | authorize <PROVIDER>
omniroute-rs memory list | add <KEY> <CONTENT> | search <QUERY>
omniroute-rs skills list
omniroute-rs db status | import <PATH> | export [FILE] [--include-secrets] | backup [FILE]
omniroute-rs version
omniroute-rs help
```

---

## 11. Environment Variables Reference

| Variable | Default | Description |
|---|---|---|
| `OMNIROUTE_PORT` | `20128` | HTTP Gateway & Dashboard listen port |
| `OMNIROUTE_DATA_DIR` | `./data` | Directory for SQLite (`omniroute-rs.db`), MITM CA, and backups |
| `OMNIROUTE_MITM_PORT` | *(unset)* | Auto-start MITM proxy on this port when running `serve` |
| `OMNIROUTE_DEFAULT_STRATEGY` | `priority` | Default routing strategy (any of 22 `v3.8.55` strategies) |
| `RUST_LOG` | `info` | Tracing log filter (`error`, `warn`, `info`, `debug`, `trace`) |
| `GUARDRAILS_ENABLED` | `true` | Enable prompt-injection guardrail |
| `PII_REDACTION_ENABLED` | `false` | Enable PII detection & redaction |
| `PII_VAULT_ENABLED` | `false` | Reversible PII masking (restored on response) |
| `AGENT_GUARD_MODE` | `off` | `agent-guard` mode: `off`, `monitor`, or `block` |
| `OMNIROUTE_COMPRESSION_ENABLED` | `false` | Enable RTK + Caveman request compression |
| `OMNIROUTE_RTK_INTENSITY` | `standard` | `off`, `light`, `standard`, `aggressive` |
| `OMNIROUTE_CAVEMAN_INTENSITY` | `off` | `off`, `lite`, `full`, `ultra` |
| `OMNIROUTE_SEMANTIC_CACHE_ENABLED` | `false` | Enable Dual-Layer Semantic Cache |
| `OMNIROUTE_SEMANTIC_CACHE_THRESHOLD` | `0.82` | Cosine similarity threshold (`0.1`–`1.0`) |
| `OMNIROUTE_GZIP_REQUEST_BODIES` | `1` | Set `0` to disable Anthropic gzip request bodies (`>= 4096B`) |
| `OMNIROUTE_CODEX_PLAN_RESPONSE_HEADER` | `0` | Set `1` to emit `X-OmniRoute-Selected-Connection-Plan` |
| `CLIENT_RETRY_AFTER_MAX_SECONDS` | `300` | Maximum `Retry-After` seconds forwarded to clients |
| `BG_REFRESH_GOOGLE_DELAY_MS` | `0` | Serialized slot spacing between Google OAuth background refreshes |
| `TOKEN_REFRESH_RETRY_JITTER` | `0` | Set `1` for deterministic SHA-256 75–125% retry jitter |
| `HEALTHCHECK_PER_PROVIDER_CONCURRENCY` | `0` | Max concurrent proactive refreshes per provider (`0` = unlimited) |
| `OMNIROUTE_HEALTHCHECK_SKIP_PROVIDERS` | *(empty)* | Comma-separated providers skipped by proactive sweep (e.g. `codex,openai`) |
| `OMNIROUTE_DISABLE_TOKEN_HEALTHCHECK` | `0` | Set `1` to disable background OAuth refresh scheduler |

---

## 12. API Endpoints Reference

### Core & Extended Inference (`/v1/*`, `/v1beta/*`)
| Route | Description |
|---|---|
| `POST /v1/chat/completions` | OpenAI Chat (JSON + SSE) with guardrails, RTK+Caveman, cache, combo & failover |
| `POST /v1/messages` & `/v1/messages/count_tokens` | Anthropic Messages API (full JSON & SSE translation) |
| `POST /v1/responses` & `/v1/responses/input_tokens` | OpenAI Responses API (JSON & SSE) |
| `POST /v1beta/models/{*spec}` | Native Gemini (`:generateContent` & `:streamGenerateContent`) |
| `POST /v1/completions` | Legacy OpenAI text completions |
| `POST /v1/embeddings`, `/v1/images/*`, `/v1/audio/*`, `/v1/rerank`, `/v1/moderations` | Multimodal & utility proxy endpoints |
| `POST /v1/decisions` | Native OpenAI Decisions API `v3.8.55` |
| `GET/POST /v1/search` & `POST /v1/web/fetch` | 8 Search providers & SSRF-guarded web fetch |
| `GET /v1/models`, `/v1/models/{*model}` | 394-provider / 3,239-model catalog + combos |
| `GET /v1/me/status`, `/v1/me/usage`, `/v1/me/quota` | Per-key self-service telemetry |

### Management, OAuth, MITM, A2A, Memory/Skills & DB (`/api/*`)
| Route | Description |
|---|---|
| `GET /`, `/dashboard`, `/healthz`, `/api/health` | Embedded Web Dashboard SPA, liveness, and JSON telemetry |
| `GET/POST /api/providers`, `GET/DELETE /api/providers/{id}` | List 394 providers + custom provider CRUD |
| `GET/POST /api/providers/credentials`, `DELETE /api/providers/credentials/{p}/{c}` | Multi-account credential management |
| `GET/POST /api/combos`, `GET/DELETE /api/combos/{id}`, `POST /api/combos/simulate` | Combo CRUD & dry-run route simulator |
| `GET/POST /api/keys`, `DELETE /api/keys/{key}` | API key CRUD with expiry, 5h budget & RPM limits |
| `GET/PUT /api/settings`, `/api/settings/compression`, `/api/settings/cache` | Gateway, RTK+Caveman & Semantic Cache config |
| `GET /api/cache/stats`, `POST /api/cache/clear`, `GET /api/usage` | Cache telemetry & usage analytics |
| `GET /api/guardrails/status`, `POST /api/guardrails/patterns`, `GET /api/gamification/status` | Security guardrails, audit events & XP/badges |
| `GET /api/oauth/providers`, `POST /api/oauth/{p}/authorize\|callback\|poll\|import`, `POST /api/oauth/{p}/{c}/refresh`, `POST /api/oauth/sweep` | OAuth 2.0 flows & background refresh scheduler |
| `GET /api/mitm/status\|ca.crt`, `POST /api/mitm/start\|stop\|regenerate-cert`, `PUT /api/mitm/bypass` | Live TLS/CONNECT MITM proxy control plane |
| `GET /.well-known/agent.json`, `POST /a2a`, `GET /api/a2a/status\|tasks` | A2A v0.3 (+ v1.0 compat) protocol & tasks |
| `GET/POST /api/memory`, `POST /api/memory/retrieve`, `GET/POST /api/skills`, `POST /api/skills/{id}/execute` | Persistent Agent Memory & Skills Registry |
| `GET /api/db/status\|export`, `POST /api/db/import/json\|upstream-sqlite`, `POST /api/db/backup` | SQLite WAL diagnostics, upstream migration & hot backup |
| `POST /api/mcp` | JSON-RPC 2.0 MCP Server (**24 tools**) |

---

## 13. Syncing the Provider Catalog from Upstream OmniRoute

Whenever a new upstream OmniRoute release lands:
```bash
git clone --depth 1 --branch release/v3.8.55 https://github.com/cheaper-inference/OmniRoute.git repo
bun --cwd repo ../scripts/sync_catalog.ts
cargo test --workspace
cargo build --release -p omni-server
```

---

## 14. Workspace Architecture (14 Crates)

| Crate | Role |
|---|---|
| `omni-types` | OpenAI/Claude/Gemini wire types, SSE parser + ANSI strip, error sanitization |
| `omni-catalog` | Embedded 394-provider & 3,239-model catalog (`v3.8.55`), `provider/model` resolver, URL builder |
| `omni-translate` | Multi-protocol translation: OpenAI Chat ↔ Claude ↔ Responses ↔ Gemini + Antigravity `v1internal` |
| `omni-compress` | RTK compression (error-preserving `#21081`) + Caveman (EN/ID) + Adaptive Context Budget |
| `omni-cache` | Dual-layer Semantic Cache (exact SHA-256 + sparse TF cosine, key-isolated, TTL + LRU) |
| `omni-oauth` | OAuth 2.0 PKCE (`S256`), RFC 8628 Device-Code, Copilot token mint, refresh jitter & Google slot |
| `omni-router` | 22 public `v3.8.55` strategies + `quota-share` + 11 auto-routing strategies |
| `omni-exec` | HTTP dispatcher + Anthropic gzip `>= 4096B` + 415 latch + 7 priority executors (incl. AWS SigV4) |
| `omni-store` | SQLite WAL (15 tables) + upstream `storage.sqlite`/`db.json` importer + `VACUUM INTO` hot backup |
| `omni-guard` | Prompt injection, PII masker, reversible `PiiVault`, `agent-guard` (`off`/`monitor`/`block`) |
| `omni-gami` | XP curve, levels, tiers, daily streaks, badge engine |
| `omni-mcp` | JSON-RPC 2.0 MCP Server (stdio & HTTP) with 24 tools |
| `omni-mitm` | Live TCP/TLS HTTP CONNECT MITM proxy + pure-Rust ECDSA P-256 Root CA & dynamic leaf certs |
| `omni-server` | Single `omniroute-rs` binary (Axum + Embedded Web Dashboard + A2A + Memory/Skills + CLI) |

---

## 15. Troubleshooting

- **Port `20128` already in use (`Address already in use`)**: inspect active listeners via `ss -tulpn | grep 20128` (Linux) or start on another port with `omniroute-rs serve --port 20129`.
- **Dashboard returns `401 Unauthorized`**: at least one API key exists in SQLite; enter it in the top-right *Gateway API Key* input and click **Save Key**.
- **Anthropic/Claude rejects gzip (`415`)**: `omniroute-rs` automatically retries uncompressed and latches the origin for 1 hour; or disable explicitly via `OMNIROUTE_GZIP_REQUEST_BODIES=0`.
- **Quick health diagnostics**: run `omniroute-rs doctor` and `omniroute-rs db status`.

---

## License

MIT — aligned with upstream OmniRoute.
