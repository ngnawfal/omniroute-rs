// Typed API client for omniroute-rs backend

export const API_KEY_STORAGE = "omniroute_api_key";

export function getStoredApiKey(): string {
  try {
    return localStorage.getItem(API_KEY_STORAGE) || "";
  } catch {
    return "";
  }
}

export function setStoredApiKey(key: string) {
  try {
    const trimmed = key.trim();
    if (trimmed) localStorage.setItem(API_KEY_STORAGE, trimmed);
    else localStorage.removeItem(API_KEY_STORAGE);
  } catch {
    // ignore storage errors
  }
}

export interface ApiResult<T = unknown> {
  ok: boolean;
  status: number;
  headers: Headers;
  data: T;
  error?: string;
}

export async function api<T = any>(
  path: string,
  opts: RequestInit = {}
): Promise<ApiResult<T>> {
  const headers = new Headers(opts.headers || {});
  const key = getStoredApiKey();
  if (key && !headers.has("Authorization")) {
    headers.set("Authorization", `Bearer ${key}`);
  }
  const res = await fetch(path, { ...opts, headers });
  const text = await res.text();
  let parsed: any = text;
  try {
    parsed = text ? JSON.parse(text) : {};
  } catch {
    // plain text body
  }
  const errMsg = !res.ok
    ? (typeof parsed === "object" && parsed?.error
        ? typeof parsed.error === "string"
          ? parsed.error
          : parsed.error?.message || JSON.stringify(parsed.error)
        : typeof parsed === "string" && parsed
          ? parsed
          : `HTTP ${res.status}`)
    : undefined;
  return {
    ok: res.ok,
    status: res.status,
    headers: res.headers,
    data: parsed as T,
    error: errMsg,
  };
}

export function apiPost<T = any>(path: string, body: unknown = {}) {
  return api<T>(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

export function apiPut<T = any>(path: string, body: unknown = {}) {
  return api<T>(path, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

export function apiDelete<T = any>(path: string) {
  return api<T>(path, { method: "DELETE" });
}

// ── Shared Types ───────────────────────────────────────────────────

export interface GatewayHealth {
  status: string;
  version: string;
  upstream_sync_version: string;
  uptime_secs: number;
  provider_count: number;
  model_count: number;
  routing_strategies: number;
  combos_count: number;
  usage?: {
    requests: number;
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
    cost_usd: number;
  };
  cache?: {
    config: {
      enabled: boolean;
      similarity_threshold: number;
      require_zero_temperature: boolean;
      max_entries: number;
      ttl_ms: number;
    };
    stats: {
      entries: number;
      direct_hits: number;
      semantic_hits: number;
      misses: number;
      hit_rate: number;
      tokens_saved: number;
    };
  };
  compression?: {
    config: {
      enabled: boolean;
      rtk_intensity: string;
      caveman_intensity: string;
    };
    stats: {
      requests_compressed: number;
      total_tokens_saved: number;
      total_compacted_turns: number;
    };
  };
  breakers?: Array<{
    provider: string;
    state: string;
    failure_streak: number;
  }>;
  active_lockouts?: Array<any>;
  oauth?: {
    supported_providers: number;
    connected_oauth_accounts: number;
    active_pending_sessions: number;
    healthcheck_disabled: boolean;
    google_refresh_delay_ms: number;
  };
}

export interface ProviderSummary {
  id: string;
  name: string;
  format: string;
  auth_type: string;
  model_count: number;
  is_free_tier: boolean;
  configured_accounts: number;
  base_url?: string;
}

export interface OAuthProviderMeta {
  id: string;
  display_name: string;
  flow: "pkce" | "device_code" | "import_only";
  uses_pkce: boolean;
  authorize_url: string;
  device_code_url: string;
  default_redirect_uri: string;
  scopes: string[];
  client_id_configured: boolean;
}

export interface CredentialItem {
  provider: string;
  connection_id: string;
  auth_type: string;
  api_key_preview: string;
  label: string;
  enabled: boolean;
  health_status: string;
  expires_at?: number | null;
  last_refreshed_at?: number | null;
  has_refresh_token?: boolean;
  provider_specific_json?: Record<string, any>;
}

export interface ComboItem {
  id: string;
  name: string;
  strategy: string;
  models: Array<{ provider: string; model: string; weight?: number }>;
}

export interface GamificationStatus {
  level: number;
  tier: string;
  total_xp: number;
  xp_to_next_level: number;
  current_streak: number;
  longest_streak?: number;
  achievements_unlocked?: number;
}
