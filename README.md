# omniroute-rs

Port Rust dari AI Gateway [OmniRoute](https://github.com/cheaper-inference/OmniRoute) (tersinkron dengan upstream **`release/v3.8.55`**, **394 provider**, **3.239 model**, **22 strategi routing**, **OAuth 2.0 PKCE & Device-Code + background refresh**, **7 executor provider prioritas + gzip Anthropic**, kompresi **RTK + Caveman**, dan **Semantic Cache**) — satu binari native ringan tanpa Node.js atau Electron.

## Jalankan cepat

```bash
# HTTP Gateway (default port 20128, opsional --mitm-port 20443)
cargo run --release -p omni-server
./target/release/omniroute-rs serve --port 20128 --mitm-port 20443

# MCP Server over stdio (JSON-RPC 2.0, 24 tools)
./target/release/omniroute-rs mcp

# Standalone live TCP/TLS HTTP CONNECT MITM proxy
./target/release/omniroute-rs mitm --port 20443 --gateway http://127.0.0.1:20128

# Utilitas CLI lengkap
./target/release/omniroute-rs doctor
./target/release/omniroute-rs status
./target/release/omniroute-rs providers --free
./target/release/omniroute-rs models --provider anthropic
./target/release/omniroute-rs combos simulate openai/gpt-6.1-sol --strategy jev
./target/release/omniroute-rs keys create --label dev --rpm 120
./target/release/omniroute-rs oauth list
./target/release/omniroute-rs memory add stack "Rust Axum + SQLite WAL"
./target/release/omniroute-rs db status
./target/release/omniroute-rs db import /path/to/storage.sqlite
./target/release/omniroute-rs db backup
./target/release/omniroute-rs version
```

## Sinkronisasi katalog dari upstream OmniRoute

```bash
git -C repo fetch --depth 1 origin release/v3.8.55 && git -C repo reset --hard FETCH_HEAD
bun --cwd repo ../scripts/sync_catalog.ts
cargo build --release
```

## Endpoint tersedia

### Core & Extended Gateway (`/v1/*` + `/v1beta/*`)
| Route | Fungsi |
|---|---|
| `POST /v1/chat/completions` | OpenAI Chat (JSON + SSE) dengan guardrails, RTK+Caveman, semantic cache, combo & failover |
| `POST /v1/messages` & `/v1/messages/count_tokens` | Anthropic Messages API (translasi penuh JSON & SSE Anthropic) |
| `POST /v1/responses` & `/v1/responses/input_tokens` | OpenAI Responses API (JSON & SSE `response.created`/`output_text.delta`/`completed`) |
| `POST /v1beta/models/{*spec}` | Native Gemini API (`:generateContent` & `:streamGenerateContent`) |
| `POST /v1/completions` | Legacy OpenAI text completions |
| `POST /v1/embeddings` | Embeddings proxy dengan failover & usage logging |
| `POST /v1/images/generations` & `/v1/images/edits` | Image generation & editing proxy |
| `POST /v1/audio/speech` & `/v1/audio/transcriptions` | TTS (raw audio) & STT (multipart passthrough) proxy |
| `POST /v1/rerank` & `/v1/moderations` | Reranking & moderations proxy |
| `POST /v1/decisions` | Native OpenAI Decisions API `v3.8.55` (`openai/gpt-6-luna`, size-capped + guardrailed) |
| `GET/POST /v1/search` & `POST /v1/web/fetch` | 8 Search providers (Serper, Brave, Perplexity, Exa, Tavily, Nimble, Keenable, DuckDuckGo) & SSRF-guarded web fetch |
| `GET /v1/models` & `GET /v1/models/{*model}` | Katalog 394 provider / 3.239 model + combo |
| `GET /v1/me/status`, `/v1/me/usage`, `/v1/me/quota` | Self-service status, analitik usage, dan quota per API key |

### Management API & Observability (`/api/*`)
| Route | Fungsi |
|---|---|
| `GET /healthz`, `GET /api/health`, `GET /`, `GET /dashboard` | Liveness, telemetri JSON, dan **Embedded Interactive Web Dashboard SPA** (8 tab, 0 aset eksternal) |
| `GET/POST /api/providers`, `GET/DELETE /api/providers/{id}` | Daftar 394 provider + CRUD custom provider |
| `GET/POST /api/providers/credentials` & `DELETE` | Multi-account credential management |
| `GET/POST /api/combos`, `GET/DELETE /api/combos/{id}`, `POST /api/combos/simulate` | Combo CRUD (22 strategi v3.8.55 + auto-combo) & dry-run |
| `GET/POST /api/keys` & `DELETE /api/keys/{key}` | API key CRUD dengan expiry, `budget_5h_usd`, `rpm_limit` |
| `GET/PUT /api/settings` | Konfigurasi gateway persisten |
| `GET/PUT /api/settings/compression` | Konfigurasi & statistik RTK + Caveman + Adaptive Context Budget |
| `GET /api/cache/stats`, `PUT /api/settings/cache`, `POST /api/cache/clear` | Konfigurasi & telemetri Semantic Cache (hit-rate, tokens saved) |
| `GET /api/usage` | Analitik usage & token (termasuk cache read/write tokens) |
| `GET /api/guardrails/status` & `POST /api/guardrails/patterns` | Prompt injection, PII masking / `PiiVault`, `agent-guard` |
| `GET /api/gamification/status` | XP, level, tier, streak harian, dan badge |
| `GET /api/mitm/status\|ca.crt`, `POST /api/mitm/start\|stop\|regenerate-cert`, `PUT /api/mitm/bypass` | Live TCP/TLS HTTP CONNECT MITM proxy (ECDSA P-256 CA + dynamic leaf cert) |
| `GET /.well-known/agent.json`, `POST /a2a`, `GET /api/a2a/status\|tasks` | Protokol **A2A v0.3** (+ kompatibilitas **v1.0** `SendMessage`/`TASK_STATE_*`, isolasi owner) |
| `GET/POST /api/memory`, `POST /api/memory/retrieve`, `GET/POST /api/skills`, `POST /api/skills/{id}/execute` | Persistent Agent Memory (TF-cosine retrieval) & Skills Registry |
| `GET /api/oauth/providers`, `POST /api/oauth/{provider}/authorize\|callback\|poll\|import` | OAuth 2.0 PKCE & Device-Code (17 provider) + token import |
| `POST /api/oauth/{provider}/{connection_id}/refresh` & `POST /api/oauth/sweep` | Manual refresh & background sweep (`BG_REFRESH_GOOGLE_DELAY_MS`, `TOKEN_REFRESH_RETRY_JITTER`, `HEALTHCHECK_PER_PROVIDER_CONCURRENCY`) |
| `GET /api/db/status`, `GET /api/db/export`, `POST /api/db/import/json\|upstream-sqlite`, `POST /api/db/backup` | Diagnostik SQLite WAL, migrasi langsung dari `storage.sqlite` / `db.json` Node.js upstream, export snapshot & hot backup |
| `POST /api/mcp` | JSON-RPC 2.0 MCP Server over HTTP (**24 tools aktif**) |

## Arsitektur Workspace (14 Crates)

| Crate | Peran |
|---|---|
| `omni-types` | Wire types OpenAI/Claude/Gemini, SSE parser + ANSI strip, error sanitization + `CLIENT_RETRY_AFTER_MAX_SECONDS` |
| `omni-catalog` | Katalog tertanam 394 provider & 3.239 model (`v3.8.55`), `provider/model` resolver, URL builder |
| `omni-translate` | Translasi multi-protokol: OpenAI Chat ↔ Claude ↔ Responses ↔ Gemini + Antigravity `v1internal` + legacy completions |
| `omni-compress` | Kompresi **RTK** (command-aware + error-preserving `#21081`) + **Caveman** (EN + ID) + **Adaptive Context Budget** & compaction |
| `omni-cache` | Dual-layer **Semantic Cache** (SHA-256 direct + sparse TF cosine similarity, key-isolated, TTL + LRU) |
| `omni-oauth` | OAuth 2.0 PKCE (`S256`), RFC 8628 Device-Code, GitHub Copilot token exchange, Google refresh slot spacing, SHA-256 retry jitter, refresh breaker, AWS region SSRF guard |
| `omni-router` | 22 strategi publik `v3.8.55` (`expiry-first`, `jev`, dll.) + `quota-share` + 11 auto-strategy (`decision`, dll.) |
| `omni-exec` | HTTP dispatcher + gzip body Anthropic `>= 4096B` + 415 fallback latch + 7 executor prioritas (Claude, Codex/ChatGPT, Copilot, Antigravity/Gemini CLI, AWS Bedrock SigV4+Converse, GCP Vertex, Azure Foundry/Classic) |
| `omni-store` | SQLite WAL (**15 tabel persisten** + importer langsung `storage.sqlite` / `db.json` upstream, snapshot export & `VACUUM INTO` hot backup) |
| `omni-guard` | Prompt injection (precompiled regex), PII masker, reversible `PiiVault`, `agent-guard` (`monitor`/`block`) |
| `omni-gami` | XP, kurva level, tier, streak harian, badge evaluation |
| `omni-mcp` | JSON-RPC 2.0 MCP Server (stdio & HTTP) dengan **24 tools** aktif |
| `omni-mitm` | Live TCP/TLS HTTP CONNECT MITM proxy + pure-Rust ECDSA P-256 Root CA & dynamic leaf cert + 10-agent targets registry |
| `omni-server` | Binary tunggal `omniroute-rs` (Axum + Embedded Web Dashboard + A2A v0.3/v1.0 + Memory/Skills + OAuth scheduler + full CLI + semua 13 library crate) |

## Status Migrasi (Fase 1–6 Selesai)

Seluruh 6 fase migrasi terhadap upstream OmniRoute **`release/v3.8.55`** telah selesai diimplementasikan dan terverifikasi dalam satu binari native Rust tanpa Node.js/Electron.

## Build & Test

```bash
cargo test --workspace
cargo build --release -p omni-server
```

## Lisensi

MIT — sesuai OmniRoute asli.
