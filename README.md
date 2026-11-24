# omniroute-rs

Port Rust dari AI Gateway [OmniRoute](https://github.com/cheaper-inference/OmniRoute) (tersinkron dengan upstream **`release/v3.8.55`**, **394 provider**, **3.239 model**, **22 strategi routing**, **OAuth 2.0 PKCE & Device-Code + background refresh**, **7 executor provider prioritas + gzip Anthropic**, kompresi **RTK + Caveman**, dan **Semantic Cache**) — satu binari native ringan tanpa Node.js atau Electron.

## Jalankan cepat

```bash
# HTTP Gateway (default port 20128)
cargo run --release -p omni-server
# atau dengan env:
OMNIROUTE_PORT=20128 OMNIROUTE_DATA_DIR=./data ./target/release/omniroute-rs

# MCP Server over stdio (JSON-RPC 2.0, 21 tools)
./target/release/omniroute-rs mcp

# Cek versi & jumlah katalog tersinkron
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
| `GET /healthz`, `GET /api/health`, `GET /` | Liveness, telemetri JSON lengkap, dan status page HTML |
| `GET/POST /api/providers`, `GET/DELETE /api/providers/{id}` | Daftar 394 provider + CRUD custom provider |
| `GET/POST /api/providers/credentials` & `DELETE` | Multi-account credential management |
| `GET/POST /api/combos`, `GET/DELETE /api/combos/{id}`, `POST /api/combos/simulate` | Combo CRUD (22 strategi v3.8.55 + auto-combo) & dry-run |
| `GET/POST /api/keys` & `DELETE /api/keys/{key}` | API key CRUD dengan expiry, `budget_5h_usd`, `rpm_limit` |
| `GET/PUT /api/settings` | Konfigurasi gateway persisten |
| `GET/PUT /api/settings/compression` | Konfigurasi & statistik RTK + Caveman + Adaptive Context Budget |
| `GET /api/cache/stats`, `PUT /api/settings/cache`, `POST /api/cache/clear` | Konfigurasi & telemetri Semantic Cache (hit-rate, tokens saved) |
| `GET /api/usage` | Analitik usage & token (termasuk cache read/write tokens) |
| `GET /api/guardrails/status` & `POST /api/guardrails/patterns` | Prompt injection, PII masking / `PiiVault`, `agent-guard` |
| `GET /api/gamification/status` & `GET /api/mitm/targets` | XP/level/streak/badges & registry target MITM |
| `GET /api/oauth/providers`, `POST /api/oauth/{provider}/authorize\|callback\|poll\|import` | OAuth 2.0 PKCE & Device-Code (17 provider) + token import |
| `POST /api/oauth/{provider}/{connection_id}/refresh` & `POST /api/oauth/sweep` | Manual refresh & background sweep (`BG_REFRESH_GOOGLE_DELAY_MS`, `TOKEN_REFRESH_RETRY_JITTER`, `HEALTHCHECK_PER_PROVIDER_CONCURRENCY`) |
| `POST /api/mcp` | JSON-RPC 2.0 MCP Server over HTTP (**21 tools aktif**) |

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
| `omni-store` | SQLite WAL (9 tabel persisten + OAuth token lifecycle, expiry, `health_status`, `health_check_interval_min`) |
| `omni-guard` | Prompt injection (precompiled regex), PII masker, reversible `PiiVault`, `agent-guard` (`monitor`/`block`) |
| `omni-gami` | XP, kurva level, tier, streak harian, badge evaluation |
| `omni-mcp` | JSON-RPC 2.0 MCP Server (stdio & HTTP) dengan **21 tools** aktif |
| `omni-mitm` | Registry 10 target MITM & routing decision helper |
| `omni-server` | Binary `omniroute-rs` (Axum + CORS + Trace + Auth strict + RPM limit + background OAuth refresh scheduler + semua 13 library crate) |

## Cakupan Fase Lanjutan

- **Fase 5:** Proxy TLS/HTTP CONNECT MITM nyata di `omni-mitm`, protokol A2A v0.3, Memory/Skills, dan subcommand CLI lengkap.
- **Fase 6:** Kompatibilitas penuh 253 migrasi DB upstream & embedded Web Dashboard.

## Build & Test

```bash
cargo test --workspace
cargo build --release -p omni-server
```

## Lisensi

MIT — sesuai OmniRoute asli.
