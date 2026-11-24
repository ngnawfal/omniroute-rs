// Sync upstream OmniRoute (release/v3.8.55) provider registry into
// crates/omni-catalog/data/providers.json for embedded Rust catalog loading.
//
// Run from workspace root:
//   bun --cwd repo ../scripts/sync_catalog.ts

import { writeFileSync, mkdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { REGISTRY } from "../repo/open-sse/config/providers/index.ts";
import { FREE_TIER_PROVIDER_SET } from "../repo/open-sse/config/freeTierProviders.ts";
import { AI_PROVIDERS, NOAUTH_PROVIDERS } from "../repo/src/shared/constants/providers.ts";

interface ExportedModel {
  id: string;
  name: string;
  aliases?: string[];
  context_length?: number;
  max_output_tokens?: number;
  supports_reasoning?: boolean;
  supported_thinking_efforts?: string[];
  supports_vision?: boolean;
  unsupported_params?: string[];
  target_format?: string;
}

interface ExportedProvider {
  id: string;
  alias?: string;
  name?: string;
  base_urls: string[];
  responses_base_url?: string;
  messages_url?: string;
  url_suffix?: string;
  auth_type: string;
  auth_header: string;
  auth_prefix?: string;
  chat_path: string;
  format: string;
  executor: string;
  headers: Record<string, string>;
  passthrough_models: boolean;
  model_id_prefix?: string;
  default_context_length?: number;
  models: string[];
  model_specs: ExportedModel[];
  anonymous_api_key?: string;
  timeout_ms?: number;
  is_free_tier: boolean;
}

const pkg = JSON.parse(readFileSync(resolve(import.meta.dir, "../repo/package.json"), "utf-8"));

const exported: ExportedProvider[] = [];
const seenIds = new Set<string>();

function normalizeAuthHeader(authHeader?: string, authType?: string): string {
  const h = (authHeader || "").trim().toLowerCase();
  if (h === "x-api-key" || h === "x-goog-api-key" || h === "api-key" || h === "authorization-key" || h === "bearer" || h === "none") {
    return h;
  }
  if (h === "authorization") return "bearer";
  if ((authType || "").toLowerCase() === "none" || (authType || "").toLowerCase() === "noauth") {
    return "none";
  }
  return "bearer";
}

function cleanHeaders(raw?: Record<string, string>): Record<string, string> {
  const out: Record<string, string> = {};
  if (!raw) return out;
  for (const [k, v] of Object.entries(raw)) {
    if (typeof v === "string" && v.length > 0) {
      out[k] = v;
    }
  }
  return out;
}

const aiProvidersMap = (AI_PROVIDERS || {}) as Record<string, any>;
const noauthMap = (NOAUTH_PROVIDERS || {}) as Record<string, any>;

for (const [key, entry] of Object.entries(REGISTRY) as [string, any][]) {
  const id = String(entry.id || key);
  seenIds.add(id);

  const baseUrls: string[] = [];
  if (Array.isArray(entry.baseUrls) && entry.baseUrls.length > 0) {
    for (const u of entry.baseUrls) {
      if (typeof u === "string" && u.trim()) baseUrls.push(u.trim());
    }
  } else if (typeof entry.baseUrl === "string" && entry.baseUrl.trim()) {
    baseUrls.push(entry.baseUrl.trim());
  }

  const headers = {
    ...cleanHeaders(entry.headers),
    ...cleanHeaders(entry.extraHeaders),
  };

  const modelSpecs: ExportedModel[] = [];
  const modelIds: string[] = [];
  const rawModels = Array.isArray(entry.models) ? entry.models : [];
  for (const m of rawModels) {
    if (!m || typeof m.id !== "string") continue;
    modelIds.push(m.id);
    const spec: ExportedModel = {
      id: m.id,
      name: typeof m.name === "string" ? m.name : m.id,
    };
    if (Array.isArray(m.aliases) && m.aliases.length > 0) spec.aliases = [...m.aliases];
    if (typeof m.contextLength === "number") spec.context_length = m.contextLength;
    if (typeof m.maxOutputTokens === "number") spec.max_output_tokens = m.maxOutputTokens;
    if (typeof m.supportsReasoning === "boolean") spec.supports_reasoning = m.supportsReasoning;
    if (Array.isArray(m.supportedThinkingEfforts) && m.supportedThinkingEfforts.length > 0) {
      spec.supported_thinking_efforts = [...m.supportedThinkingEfforts];
    }
    if (typeof m.supportsVision === "boolean") spec.supports_vision = m.supportsVision;
    if (Array.isArray(m.unsupportedParams) && m.unsupportedParams.length > 0) {
      spec.unsupported_params = [...m.unsupportedParams];
    }
    if (typeof m.targetFormat === "string") spec.target_format = m.targetFormat;
    modelSpecs.push(spec);
  }

  const passthrough = Boolean(entry.passthroughModels);
  if (passthrough && !modelIds.includes("*")) {
    modelIds.push("*");
  }

  const meta = aiProvidersMap[id] || noauthMap[id] || (entry.alias ? aiProvidersMap[entry.alias] : undefined);
  const isFree =
    FREE_TIER_PROVIDER_SET.has(id) ||
    (typeof entry.alias === "string" && FREE_TIER_PROVIDER_SET.has(entry.alias)) ||
    Boolean(entry.anonymousApiKey) ||
    Boolean(noauthMap[id]);

  const item: ExportedProvider = {
    id,
    base_urls: baseUrls,
    auth_type: String(entry.authType || "apikey"),
    auth_header: normalizeAuthHeader(entry.authHeader, entry.authType),
    chat_path: typeof entry.chatPath === "string" ? entry.chatPath : "",
    format: String(entry.format || "openai"),
    executor: String(entry.executor || "default"),
    headers,
    passthrough_models: passthrough,
    models: modelIds,
    model_specs: modelSpecs,
    is_free_tier: isFree,
  };

  if (entry.alias && entry.alias !== id) item.alias = String(entry.alias);
  if (meta?.name) item.name = String(meta.name);
  if (typeof entry.responsesBaseUrl === "string") item.responses_base_url = entry.responsesBaseUrl;
  if (typeof entry.messagesUrl === "string") item.messages_url = entry.messagesUrl;
  if (typeof entry.urlSuffix === "string") item.url_suffix = entry.urlSuffix;
  if (typeof entry.authPrefix === "string") item.auth_prefix = entry.authPrefix;
  if (typeof entry.modelIdPrefix === "string") item.model_id_prefix = entry.modelIdPrefix;
  if (typeof entry.defaultContextLength === "number") item.default_context_length = entry.defaultContextLength;
  if (typeof entry.anonymousApiKey === "string") item.anonymous_api_key = entry.anonymousApiKey;
  if (typeof entry.timeoutMs === "number") item.timeout_ms = entry.timeoutMs;

  exported.push(item);
}

// Include any additional providers present in AI_PROVIDERS / NOAUTH_PROVIDERS not in REGISTRY
for (const [srcName, mapObj] of [["AI_PROVIDERS", aiProvidersMap], ["NOAUTH_PROVIDERS", noauthMap]] as const) {
  for (const [k, meta] of Object.entries(mapObj)) {
    const id = String(meta?.id || k);
    if (seenIds.has(id)) continue;
    // Check if already covered by alias
    const byAlias = exported.find((p) => p.alias === id || p.alias === k);
    if (byAlias) {
      if (!byAlias.name && meta?.name) byAlias.name = String(meta.name);
      continue;
    }
    seenIds.add(id);
    const baseUrls: string[] = [];
    if (typeof meta?.baseUrl === "string" && meta.baseUrl.trim()) {
      baseUrls.push(meta.baseUrl.trim());
    }
    exported.push({
      id,
      alias: meta?.alias && meta.alias !== id ? String(meta.alias) : undefined,
      name: meta?.name ? String(meta.name) : id,
      base_urls: baseUrls,
      auth_type: srcName === "NOAUTH_PROVIDERS" ? "none" : String(meta?.authType || "apikey"),
      auth_header: srcName === "NOAUTH_PROVIDERS" ? "none" : "bearer",
      chat_path: "",
      format: String(meta?.format || "openai"),
      executor: "default",
      headers: {},
      passthrough_models: Boolean(meta?.passthroughModels),
      models: meta?.passthroughModels ? ["*"] : [],
      model_specs: [],
      is_free_tier: FREE_TIER_PROVIDER_SET.has(id) || srcName === "NOAUTH_PROVIDERS",
    });
  }
}

exported.sort((a, b) => a.id.localeCompare(b.id));

const outDir = resolve(import.meta.dir, "../crates/omni-catalog/data");
mkdirSync(outDir, { recursive: true });
const outPath = resolve(outDir, "providers.json");
const payload = {
  upstream_version: pkg.version,
  provider_count: exported.length,
  model_count: exported.reduce((sum, p) => sum + p.model_specs.length, 0),
  free_tier_count: exported.filter((p) => p.is_free_tier).length,
  providers: exported,
};

writeFileSync(outPath, JSON.stringify(payload));
console.log(
  `Synced OmniRoute v${payload.upstream_version}: ${payload.provider_count} providers, ${payload.model_count} models, ${payload.free_tier_count} free-tier -> ${outPath} (${(JSON.stringify(payload).length / 1024).toFixed(1)} KB)`
);
