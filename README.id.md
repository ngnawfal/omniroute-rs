# omniroute-rs

> **Bahasa:** [English](README.md) | **Bahasa Indonesia**

Port native **Rust** dari AI Gateway [OmniRoute](https://github.com/cheaper-inference/OmniRoute) yang tersinkron dengan upstream **`release/v3.8.55`** — dikemas sebagai **satu binari tunggal (~10–11 MB)** dengan konsumsi RAM sangat rendah (~15–25 MB RSS), tanpa Node.js, tanpa Electron, dan tanpa aset frontend eksternal. Cocok untuk VPS kecil, mini PC, Raspberry Pi, hingga STB Armbian (`aarch64`, RAM 1 GB seperti HG680P).

---

## Fitur Utama

- **394 Provider & 3.239 Model** (termasuk **82 provider free-tier**) tertanam langsung di binari (`v3.8.55`).
- **5 Wire Surface Inferensi**: OpenAI Chat (`/v1/chat/completions`), Anthropic Messages (`/v1/messages`), OpenAI Responses (`/v1/responses`), Native Gemini (`/v1beta/models/*`), dan Legacy Completions (`/v1/completions`).
- **Extended Multimodal & Utility APIs**: `/v1/embeddings`, `/v1/images/*`, `/v1/audio/*`, `/v1/rerank`, `/v1/moderations`, `/v1/decisions` (`v3.8.55`), `/v1/search` (8 mesin pencari), `/v1/web/fetch` (SSRF-guarded), dan `/v1/me/*`.
- **22 Strategi Routing & Resiliensi**: `priority`, `round-robin`, `weighted`, `random`, `least-used`, `cost-optimized`, `strict-random`, `p2c`, `lkgp`, `jev`, `expiry-first`, `fill-first`, `auto` (11 sub-strategi), circuit breaker 4-state, model lockout, dan `Retry-After` parser.
- **OAuth 2.0 (17 Provider) & 7 Executor Prioritas**:
  - PKCE `S256` & RFC 8628 Device-Code + background refresh scheduler (`BG_REFRESH_GOOGLE_DELAY_MS`, `TOKEN_REFRESH_RETRY_JITTER=1`, `HEALTHCHECK_PER_PROVIDER_CONCURRENCY`).
  - Executor khusus: **Claude / Claude Code** (gzip body `>= 4096B` + auto-resend fallback `415`, conditional `skills-2025-10-02`), **Codex / ChatGPT**, **GitHub Copilot / GHE** (auto-mint token, Claude → `/v1/messages`, Codex → `/responses`), **Antigravity & Gemini CLI** (`v1internal` + multi-endpoint fallback), **AWS Bedrock** (pure-Rust **AWS SigV4** + Converse API), **GCP Vertex AI** (Express & project-scoped), dan **Azure OpenAI / AI Foundry**.
- **Optimasi Token & Biaya**: kompresi **RTK** (error-preserving `#21081`) + **Caveman** (EN & ID) + **Adaptive Context Budget** + **Dual-Layer Semantic Cache** (exact SHA-256 + sparse cosine similarity, terisolasi per API key).
- **Keamanan & Gamifikasi**: Prompt Injection guardrail, PII masking + reversible `PiiVault`, `agent-guard` (`off`/`monitor`/`block`), serta sistem XP, level, streak, dan badge.
- **Live TCP/TLS HTTP CONNECT MITM Proxy**: pure-Rust ECDSA P-256 Root CA + dynamic leaf cert per-host (`rustls`, tanpa OpenSSL) untuk 10 agen AI (Claude Code, Codex, Copilot, Antigravity, Cursor, Kiro, Zed, OpenCode, Trae, GHE Copilot).
- **Protokol A2A v0.3 (+ v1.0 Compat)**, **Persistent Agent Memory** (SQLite WAL + TF-cosine retrieval), **Skills Registry**, dan **24 MCP Tools** (stdio & HTTP).
- **Embedded Web Dashboard SPA (8 Tab)** di `/` & `/dashboard` + **Importer langsung `storage.sqlite` / `db.json`** dari OmniRoute Node.js lama.

---

## Daftar Isi

1. [Prasyarat & Instalasi (Build dari Source)](#1-prasyarat--instalasi-build-dari-source)
2. [Cross-Compile ke Linux ARM64 / STB (HG680P / Armbian / Raspberry Pi)](#2-cross-compile-ke-linux-arm64--stb-hg680p--armbian--raspberry-pi)
3. [Deploy Permanen sebagai Service `systemd` (Linux / STB)](#3-deploy-permanen-sebagai-service-systemd-linux--stb)
4. [Menjalankan Gateway & Membuka Web Dashboard](#4-menjalankan-gateway--membuka-web-dashboard)
5. [Konfigurasi Client (OpenAI SDK, Claude Code, Cursor, Cline, OpenCode)](#5-konfigurasi-client-openai-sdk-claude-code-cursor-cline-opencode)
6. [Setup OAuth 2.0, Provider Cloud (Bedrock, Vertex, Azure) & Combo](#6-setup-oauth-20-provider-cloud-bedrock-vertex-azure--combo)
7. [Mengaktifkan MITM TLS Proxy & Trust Root CA](#7-mengaktifkan-mitm-tls-proxy--trust-root-ca)
8. [Migrasi Data dari OmniRoute Node.js Lama (`storage.sqlite` / `db.json`)](#8-migrasi-data-dari-omniroute-nodejs-lama-storagesqlite--dbjson)
9. [Integrasi MCP Server (24 Tools) & A2A v0.3 / v1.0](#9-integrasi-mcp-server-24-tools--a2a-v03--v10)
10. [Referensi CLI Lengkap](#10-referensi-cli-lengkap)
11. [Referensi Environment Variables](#11-referensi-environment-variables)
12. [Referensi API Endpoints](#12-referensi-api-endpoints)
13. [Sinkronisasi Katalog dari Upstream OmniRoute](#13-sinkronisasi-katalog-dari-upstream-omniroute)
14. [Arsitektur Workspace (14 Crates)](#14-arsitektur-workspace-14-crates)
15. [Troubleshooting](#15-troubleshooting)

---

## 1. Prasyarat & Instalasi (Build dari Source)

### Prasyarat
- **Rust toolchain** `>= 1.80` (`rustup` + `cargo`)
- **C compiler** (untuk kompilasi `rusqlite` bundled SQLite & `ring`):
  - **Linux (Ubuntu/Debian/Armbian)**: `sudo apt update && sudo apt install -y build-essential git`
  - **Windows**: MinGW-w64 (`g++` / target `x86_64-pc-windows-gnu`, sudah dikonfigurasi di `.cargo/config.toml`) atau Visual Studio Build Tools (`msvc`).
  - **macOS**: `xcode-select --install`

### Clone & Build

#### Linux / macOS
```bash
git clone https://github.com/ngnawfal/omniroute-rs.git
cd omniroute-rs

# Jalankan seluruh test (102 test across 14 crates)
cargo test --workspace

# Build binari release (override default Windows target jika di Linux/macOS)
cargo build --release --target "$(rustc -vV | awk '/host:/ {print $2}')" -p omni-server

# Lokasi binari hasil build:
ls -lh target/*/release/omniroute-rs
```

#### Windows (PowerShell)
```powershell
git clone https://github.com/ngnawfal/omniroute-rs.git
cd omniroute-rs

# Test & build release
cargo test --workspace
cargo build --release -p omni-server

# Cek versi binari
.\target\x86_64-pc-windows-gnu\release\omniroute-rs.exe version
```

---

## 2. Cross-Compile ke Linux ARM64 / STB (HG680P / Armbian / Raspberry Pi)

Perangkat hemat daya seperti STB **HG680P** (RAM ~787 MB, eMMC kecil) sangat ideal menjalankan `omniroute-rs`, namun **tidak disarankan meng-compile Rust langsung di STB** karena `cargo` membutuhkan RAM/disk besar saat build. Cara tercepat adalah cross-compile di PC (x86_64) via **Docker**, lalu kirim satu file binari (~10 MB) ke STB.

### Cross-Compile Cepat via Docker (dari Windows / Linux / macOS x86_64)
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

Lalu kirim ke STB via `scp`:
```bash
scp dist/omniroute-rs-aarch64 root@192.168.11.209:/opt/omniroute-rs/omniroute-rs
```

---

## 3. Deploy Permanen sebagai Service `systemd` (Linux / STB)

Di server/STB target (`root`):

```bash
mkdir -p /opt/omniroute-rs/data
chmod 755 /opt/omniroute-rs/omniroute-rs
ln -sf /opt/omniroute-rs/omniroute-rs /usr/local/bin/omniroute-rs

# File environment (ubah sesuai kebutuhan)
cat > /opt/omniroute-rs/omniroute-rs.env <<'EOF'
OMNIROUTE_PORT=20128
OMNIROUTE_DATA_DIR=/opt/omniroute-rs/data
RUST_LOG=info
# Opsional: aktifkan juga MITM proxy otomatis di port 20443
# OMNIROUTE_MITM_PORT=20443
EOF

# Unit systemd
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

### Perintah Operasional Service
```bash
# Lihat log live
journalctl -u omniroute-rs -f

# Restart setelah ubah .env
systemctl restart omniroute-rs

# Cek diagnostik & kesehatan
omniroute-rs doctor
curl -s http://127.0.0.1:20128/api/health
```

---

## 4. Menjalankan Gateway & Membuka Web Dashboard

### Jalankan Manual
```bash
# Default port 20128
omniroute-rs serve

# Custom port & langsung aktifkan MITM proxy di port 20443
omniroute-rs serve --port 20128 --mitm-port 20443
```

### Akses Web Dashboard Embedded
Buka di browser:
- Lokal: **`http://127.0.0.1:20128/dashboard`** (atau `http://127.0.0.1:20128/`)
- LAN / STB: **`http://<IP-SERVER>:20128/dashboard`** (contoh: `http://192.168.11.209:20128/dashboard`)

> **Mode Autentikasi:**
> - **Default (0 API key di database):** mode *open-door* untuk setup lokal awal — semua endpoint dapat diakses langsung.
> - **Begitu minimal 1 API key dibuat** (`omniroute-rs keys create` atau via tab *Keys & Guardrails* di Dashboard): autentikasi ketat langsung aktif; semua request `/v1/*`, `/a2a`, dan mutasi `/api/*` wajib menyertakan `Authorization: Bearer <API_KEY>` atau `x-api-key: <API_KEY>`. Simpan key tersebut di kotak *Gateway API Key* di pojok kanan atas Dashboard.

---

## 5. Konfigurasi Client (OpenAI SDK, Claude Code, Cursor, Cline, OpenCode)

### OpenAI SDK (Python / Node.js / curl)
```bash
curl http://127.0.0.1:20128/v1/chat/completions \
  -H "Authorization: Bearer sk-omni-YOUR_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "openai/gpt-6.1-sol",
    "messages": [{"role": "user", "content": "Halo dari omniroute-rs!"}]
  }'
```

### Anthropic SDK / Claude Code
```bash
export ANTHROPIC_BASE_URL="http://127.0.0.1:20128"
export ANTHROPIC_API_KEY="sk-omni-YOUR_KEY"
claude
```
Gateway secara otomatis menerjemahkan `/v1/messages` (JSON & SSE streaming, termasuk `thinking` & `tool_use`) ke provider tujuan mana pun (Anthropic, Claude OAuth, Copilot, Bedrock, OpenAI, Gemini, atau Combo).

### OpenAI Responses API & Native Gemini
- **Responses API**: `POST http://127.0.0.1:20128/v1/responses`
- **Native Gemini**: `POST http://127.0.0.1:20128/v1beta/models/gemini/gemini-2.5-pro:generateContent`

### Cursor / Cline / Roo Code / OpenCode / Continue
- **Base URL**: `http://<IP-SERVER>:20128/v1`
- **API Key**: key yang dibuat lewat `omniroute-rs keys create` (atau sembarang string bila belum ada key).
- **Model**: format `provider/model` (mis. `claude/claude-opus-4-7`, `github/claude-sonnet-4.5`, `groq/openai/gpt-oss-120b`, atau nama **Combo** buatan Anda).

---

## 6. Setup OAuth 2.0, Provider Cloud (Bedrock, Vertex, Azure) & Combo

### Tambah Kredensial API Key
Bisa lewat Dashboard (tab *Providers & OAuth*) atau `curl`:
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

### Login OAuth 2.0 (Claude Code, Codex, ChatGPT, GitHub Copilot, Antigravity, Gemini CLI, dll.)
```bash
# 1. Lihat 17 provider OAuth yang didukung
curl http://127.0.0.1:20128/api/oauth/providers

# 2. Mulai PKCE (contoh: codex) -> buka authorize_url di browser
curl -X POST http://127.0.0.1:20128/api/oauth/codex/authorize -H "Content-Type: application/json" -d '{}'

# 3. Kirim authorization code + state dari redirect URL
curl -X POST http://127.0.0.1:20128/api/oauth/codex/callback \
  -H "Content-Type: application/json" \
  -d '{"code": "CODE_DARI_CALLBACK", "state": "STATE_DARI_LANGKAH_2"}'

# Atau untuk GitHub Copilot (Device-Code Flow):
curl -X POST http://127.0.0.1:20128/api/oauth/github/authorize -H "Content-Type: application/json" -d '{}'
# Masukkan user_code di verification_uri, lalu poll:
curl -X POST http://127.0.0.1:20128/api/oauth/github/poll \
  -H "Content-Type: application/json" \
  -d '{"state": "STATE_DARI_AUTHORIZE"}'
```

### AWS Bedrock (IAM SigV4 atau Bedrock Native API Key)
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

### GCP Vertex AI & Azure OpenAI / Foundry
- **Vertex AI**: isi `provider_specific_json: {"project_id": "my-gcp-proj", "region": "us-central1"}` (OAuth Bearer) atau gunakan Express API key di `api_key` untuk Gemini.
- **Azure**: set `provider: "azure"`, `auth_type: "api-key"`, `base_urls: ["https://<resource>.openai.azure.com"]` (Classic) atau `["https://<project>.services.ai.azure.com"]` (Foundry otomatis terdeteksi).

### Buat Combo Failover (22 Strategi)
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

## 7. Mengaktifkan MITM TLS Proxy & Trust Root CA

`omniroute-rs` membawa server **HTTP CONNECT + TLS Interception MITM** native (`rustls` + ECDSA P-256 CA yang dibuat otomatis di `<data_dir>/mitm/ca.crt`).

### Menjalankan MITM Proxy
- **Lewat Dashboard**: buka tab *MITM & A2A* → klik **Start Proxy** (default port `20443`).
- **Lewat CLI standalone**:
  ```bash
  omniroute-rs mitm --port 20443 --gateway http://127.0.0.1:20128
  ```
- **Otomatis saat `serve`**: `omniroute-rs serve --port 20128 --mitm-port 20443`.

### Mempercayai Root CA di Client
Unduh sertifikat dari `http://<IP-SERVER>:20128/api/mitm/ca.crt`:
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
- **Windows**: klik dua kali `omniroute-mitm-ca.crt` → *Install Certificate* → *Trusted Root Certification Authorities*.

---

## 8. Migrasi Data dari OmniRoute Node.js Lama (`storage.sqlite` / `db.json`)

Anda dapat memindahkan akun, OAuth token, combo, API key, setting, alias, dan riwayat usage dari instalasi OmniRoute Node.js lama secara **non-destruktif** (sumber hanya dibuka read-only):

### Lewat CLI
```bash
# Import langsung dari storage.sqlite upstream
omniroute-rs db import /root/.omniroute/storage.sqlite

# Atau dari legacy db.json / snapshot JSON
omniroute-rs db import /path/to/db.json

# Cek integritas & jumlah baris di 15 tabel
omniroute-rs db status

# Hot backup SQLite (aman saat server aktif)
omniroute-rs db backup /opt/omniroute-rs/data/backups/manual.db
```

### Lewat REST API / Web Dashboard
Buka tab **Database & Migration** di Dashboard atau panggil `POST /api/db/import/upstream-sqlite` dengan `{"path": "/path/to/storage.sqlite"}`.

---

## 9. Integrasi MCP Server (24 Tools) & A2A v0.3 / v1.0

### MCP over stdio (untuk Claude Desktop / Cursor / Windsurf / Cline)
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
Atau over HTTP: `POST http://<IP-SERVER>:20128/api/mcp` (JSON-RPC 2.0).

### A2A v0.3 (+ Kompatibilitas v1.0)
- **Agent Card**: `GET http://<IP-SERVER>:20128/.well-known/agent.json`
- **JSON-RPC 2.0**: `POST http://<IP-SERVER>:20128/a2a` mendukung `message/send`, `message/stream` (SSE), `tasks/get`, `tasks/cancel`, serta alias v1.0 (`SendMessage`, `SendStreamingMessage`).

---

## 10. Referensi CLI Lengkap

```bash
omniroute-rs serve [--port N] [--mitm-port N]          # Jalankan HTTP gateway (+ opsional MITM)
omniroute-rs mcp                                       # Jalankan MCP server stdio (24 tools)
omniroute-rs mitm [--port N] [--gateway URL]           # Jalankan standalone TLS/CONNECT MITM proxy
omniroute-rs status                                    # Ringkasan JSON katalog, DB, OAuth & MITM
omniroute-rs doctor                                    # Diagnostik lengkap environment, SQLite, CA & port
omniroute-rs providers [--free] [--q TEXT] [--limit N] # Cari & daftar 394 provider
omniroute-rs models [--provider ID] [--q TEXT]         # Cari & daftar 3.239 model
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

## 11. Referensi Environment Variables

| Variable | Default | Fungsi |
|---|---|---|
| `OMNIROUTE_PORT` | `20128` | Port HTTP Gateway & Dashboard |
| `OMNIROUTE_DATA_DIR` | `./data` | Direktori SQLite (`omniroute-rs.db`), CA MITM, dan backup |
| `OMNIROUTE_MITM_PORT` | *(unset)* | Jika diisi (mis. `20443`), MITM proxy otomatis aktif saat `serve` |
| `OMNIROUTE_DEFAULT_STRATEGY` | `priority` | Strategi routing default (salah satu dari 22 strategi `v3.8.55`) |
| `RUST_LOG` | `info` | Level log (`error`, `warn`, `info`, `debug`, `trace`) |
| `GUARDRAILS_ENABLED` | `true` | Aktifkan proteksi prompt-injection |
| `PII_REDACTION_ENABLED` | `false` | Aktifkan deteksi/redaksi PII pada request |
| `PII_VAULT_ENABLED` | `false` | Masking PII reversibel (dikembalikan pada response) |
| `AGENT_GUARD_MODE` | `off` | Mode `agent-guard`: `off`, `monitor`, atau `block` |
| `OMNIROUTE_COMPRESSION_ENABLED` | `false` | Aktifkan kompresi RTK + Caveman |
| `OMNIROUTE_RTK_INTENSITY` | `standard` | `off`, `light`, `standard`, `aggressive` |
| `OMNIROUTE_CAVEMAN_INTENSITY` | `off` | `off`, `lite`, `full`, `ultra` |
| `OMNIROUTE_SEMANTIC_CACHE_ENABLED` | `false` | Aktifkan Dual-Layer Semantic Cache |
| `OMNIROUTE_SEMANTIC_CACHE_THRESHOLD` | `0.82` | Ambang kemiripan kosinus (`0.1`–`1.0`) |
| `OMNIROUTE_GZIP_REQUEST_BODIES` | `1` | Set `0` untuk menonaktifkan gzip body Anthropic `>= 4096B` |
| `OMNIROUTE_CODEX_PLAN_RESPONSE_HEADER` | `0` | Set `1` untuk menampilkan `X-OmniRoute-Selected-Connection-Plan` |
| `CLIENT_RETRY_AFTER_MAX_SECONDS` | `300` | Batas maksimum header `Retry-After` yang diteruskan ke client |
| `BG_REFRESH_GOOGLE_DELAY_MS` | `0` | Jeda serialisasi refresh OAuth Google (`antigravity`/`agy`/`gemini-cli`) |
| `TOKEN_REFRESH_RETRY_JITTER` | `0` | Set `1` untuk jitter SHA-256 deterministik 75–125% saat retry refresh |
| `HEALTHCHECK_PER_PROVIDER_CONCURRENCY` | `0` | Batas paralel refresh proaktif per provider (`0` = tanpa batas) |
| `OMNIROUTE_HEALTHCHECK_SKIP_PROVIDERS` | *(kosong)* | Daftar provider dipisah koma yang dilewati background sweep (mis. `codex,openai`) |
| `OMNIROUTE_DISABLE_TOKEN_HEALTHCHECK` | `0` | Set `1` untuk mematikan scheduler background refresh OAuth |

---

## 12. Referensi API Endpoints

### Core & Extended Inference (`/v1/*`, `/v1beta/*`)
| Route | Fungsi |
|---|---|
| `POST /v1/chat/completions` | OpenAI Chat (JSON + SSE) dengan guardrails, RTK+Caveman, cache, combo & failover |
| `POST /v1/messages` & `/v1/messages/count_tokens` | Anthropic Messages API (translasi penuh JSON & SSE Anthropic) |
| `POST /v1/responses` & `/v1/responses/input_tokens` | OpenAI Responses API (JSON & SSE) |
| `POST /v1beta/models/{*spec}` | Native Gemini (`:generateContent` & `:streamGenerateContent`) |
| `POST /v1/completions` | Legacy OpenAI text completions |
| `POST /v1/embeddings`, `/v1/images/*`, `/v1/audio/*`, `/v1/rerank`, `/v1/moderations` | Proxy multimodal & utility |
| `POST /v1/decisions` | Native OpenAI Decisions API `v3.8.55` |
| `GET/POST /v1/search` & `POST /v1/web/fetch` | 8 Search provider & web fetch ber-SSRF guard |
| `GET /v1/models`, `/v1/models/{*model}` | Katalog 394 provider / 3.239 model + combo |
| `GET /v1/me/status`, `/v1/me/usage`, `/v1/me/quota` | Self-service telemetri per API key |

### Management, OAuth, MITM, A2A, Memory/Skills & DB (`/api/*`)
| Route | Fungsi |
|---|---|
| `GET /`, `/dashboard`, `/healthz`, `/api/health` | Embedded Web Dashboard SPA, liveness, dan telemetri JSON |
| `GET/POST /api/providers`, `GET/DELETE /api/providers/{id}` | Daftar 394 provider + CRUD custom provider |
| `GET/POST /api/providers/credentials`, `DELETE /api/providers/credentials/{p}/{c}` | Multi-account credential management |
| `GET/POST /api/combos`, `GET/DELETE /api/combos/{id}`, `POST /api/combos/simulate` | Combo CRUD & dry-run simulator |
| `GET/POST /api/keys`, `DELETE /api/keys/{key}` | API key CRUD dengan expiry, budget 5 jam & RPM limit |
| `GET/PUT /api/settings`, `/api/settings/compression`, `/api/settings/cache` | Konfigurasi gateway, RTK+Caveman, dan Semantic Cache |
| `GET /api/cache/stats`, `POST /api/cache/clear`, `GET /api/usage` | Telemetri cache & analitik penggunaan |
| `GET /api/guardrails/status`, `POST /api/guardrails/patterns`, `GET /api/gamification/status` | Keamanan, audit guardrail & XP/badge |
| `GET /api/oauth/providers`, `POST /api/oauth/{p}/authorize\|callback\|poll\|import`, `POST /api/oauth/{p}/{c}/refresh`, `POST /api/oauth/sweep` | Manajemen OAuth 2.0 & refresh scheduler |
| `GET /api/mitm/status\|ca.crt`, `POST /api/mitm/start\|stop\|regenerate-cert`, `PUT /api/mitm/bypass` | Kontrol live TLS/CONNECT MITM proxy |
| `GET /.well-known/agent.json`, `POST /a2a`, `GET /api/a2a/status\|tasks` | Protokol A2A v0.3 (+ v1.0 compat) |
| `GET/POST /api/memory`, `POST /api/memory/retrieve`, `GET/POST /api/skills`, `POST /api/skills/{id}/execute` | Persistent Agent Memory & Skills Registry |
| `GET /api/db/status\|export`, `POST /api/db/import/json\|upstream-sqlite`, `POST /api/db/backup` | Diagnostik, migrasi upstream & hot backup SQLite |
| `POST /api/mcp` | JSON-RPC 2.0 MCP Server (**24 tools**) |

---

## 13. Sinkronisasi Katalog dari Upstream OmniRoute

Saat ada rilis OmniRoute baru di upstream:
```bash
git clone --depth 1 --branch release/v3.8.55 https://github.com/cheaper-inference/OmniRoute.git repo
bun --cwd repo ../scripts/sync_catalog.ts
cargo test --workspace
cargo build --release -p omni-server
```

---

## 14. Arsitektur Workspace (14 Crates)

| Crate | Peran |
|---|---|
| `omni-types` | Wire types OpenAI/Claude/Gemini, SSE parser + ANSI strip, error sanitization |
| `omni-catalog` | Katalog tertanam 394 provider & 3.239 model (`v3.8.55`), `provider/model` resolver, URL builder |
| `omni-translate` | Translasi multi-protokol: OpenAI Chat ↔ Claude ↔ Responses ↔ Gemini + Antigravity `v1internal` |
| `omni-compress` | Kompresi RTK (error-preserving `#21081`) + Caveman (EN/ID) + Adaptive Context Budget |
| `omni-cache` | Dual-layer Semantic Cache (SHA-256 direct + sparse TF cosine, key-isolated, TTL + LRU) |
| `omni-oauth` | OAuth 2.0 PKCE (`S256`), RFC 8628 Device-Code, Copilot token mint, refresh jitter & Google slot |
| `omni-router` | 22 strategi publik `v3.8.55` + `quota-share` + 11 auto-routing strategies |
| `omni-exec` | HTTP dispatcher + gzip Anthropic `>= 4096B` + 415 latch + 7 executor prioritas (termasuk AWS SigV4) |
| `omni-store` | SQLite WAL (15 tabel) + importer `storage.sqlite`/`db.json` upstream + `VACUUM INTO` hot backup |
| `omni-guard` | Prompt injection, PII masker, reversible `PiiVault`, `agent-guard` (`off`/`monitor`/`block`) |
| `omni-gami` | XP, kurva level, tier, streak harian, badge evaluation |
| `omni-mcp` | JSON-RPC 2.0 MCP Server (stdio & HTTP) dengan 24 tools |
| `omni-mitm` | Live TCP/TLS HTTP CONNECT MITM proxy + pure-Rust ECDSA P-256 Root CA & dynamic leaf certs |
| `omni-server` | Binary tunggal `omniroute-rs` (Axum + Embedded Web Dashboard + A2A + Memory/Skills + CLI) |

---

## 15. Troubleshooting

- **Port `20128` sudah terpakai (`Address already in use`)**: cek proses aktif via `ss -tulpn | grep 20128` (Linux) atau jalankan di port lain `omniroute-rs serve --port 20129`.
- **Dashboard mengembalikan `401 Unauthorized`**: Anda sudah membuat API key; masukkan key tersebut di kolom *Gateway API Key* di pojok kanan atas Dashboard lalu klik **Save Key**.
- **Request ke Anthropic/Claude menolak gzip (`415`)**: `omniroute-rs` otomatis mengirim ulang tanpa kompresi dan meng-latch origin selama 1 jam; atau set `OMNIROUTE_GZIP_REQUEST_BODIES=0`.
- **Cek kesehatan cepat**: jalankan `omniroute-rs doctor` dan `omniroute-rs db status`.

---

## Lisensi

MIT — sesuai dengan lisensi OmniRoute upstream.
