import { useEffect, useMemo, useState } from "react";
import {
  Search,
  Plus,
  Trash2,
  RefreshCw,
  KeyRound,
  Plug,
  Sparkles,
  CheckCircle2,
  ListTree,
  Copy,
  Check,
  Eye,
  Brain,
} from "lucide-react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Badge,
  Button,
  Input,
  Dialog,
  PageHeader,
  EmptyState,
} from "@/components/ui/primitives";
import { ProviderIcon } from "@/components/ProviderIcon";
import {
  api,
  apiDelete,
  apiPost,
  type CredentialItem,
  type OAuthProviderMeta,
  type ProviderSummary,
} from "@/lib/api";
import { cn, copyToClipboard, fmtRelativeExpiry } from "@/lib/utils";

const OAUTH_ALIASES: Record<string, string> = {
  "claude-code": "claude",
  anthropic: "claude",
  "openai-native": "chatgpt",
  copilot: "github",
  "github-copilot": "github",
  gemini: "gemini-cli",
  clinepass: "cline",
  "amazon-q": "kiro",
};

type FilterChip = "all" | "oauth" | "free" | "claude" | "gemini" | "openai";

export function ProvidersPage({
  credentials,
  oauthProviders,
  isDark,
  onLaunchOAuth,
  onOpenAddCred,
  onRefresh,
  onToast,
}: {
  credentials: CredentialItem[];
  oauthProviders: OAuthProviderMeta[];
  isDark: boolean;
  onLaunchOAuth: (providerId: string, displayName: string) => void;
  onOpenAddCred: (providerId?: string) => void;
  onRefresh: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<FilterChip>("all");
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [total, setTotal] = useState(394);
  const [refreshingKey, setRefreshingKey] = useState<string | null>(null);
  const [inspectProvider, setInspectProvider] = useState<{
    id: string;
    name: string;
    connectionId?: string;
  } | null>(null);

  const oauthMap = useMemo(() => {
    const m: Record<string, OAuthProviderMeta> = {};
    oauthProviders.forEach((p) => {
      m[p.id] = p;
    });
    return m;
  }, [oauthProviders]);

  useEffect(() => {
    let alive = true;
    const timer = window.setTimeout(async () => {
      const free = filter === "free" ? "true" : "false";
      const r = await api<{ total: number; providers: ProviderSummary[] }>(
        `/api/providers?q=${encodeURIComponent(query.trim())}&free_only=${free}&limit=400`
      );
      if (alive && r.ok) {
        setTotal(r.data.total ?? 0);
        setProviders(r.data.providers || []);
      }
    }, 120);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [query, filter, credentials.length]);

  const filteredProviders = useMemo(() => {
    let list = providers;
    if (filter === "oauth") {
      list = list.filter((p) => p.auth_type === "oauth" || !!oauthMap[p.id]);
    } else if (filter === "claude") {
      list = list.filter((p) => p.format === "claude");
    } else if (filter === "gemini") {
      list = list.filter((p) => p.format === "gemini");
    } else if (filter === "openai") {
      list = list.filter((p) => p.format === "openai");
    }
    return list.slice(0, 120);
  }, [providers, filter, oauthMap]);

  const handleDelete = async (prov: string, cid: string) => {
    const r = await apiDelete(
      `/api/providers/credentials/${encodeURIComponent(prov)}/${encodeURIComponent(cid)}`
    );
    if (r.ok) {
      onToast(`Disconnected ${prov} (${cid})`, "info");
      onRefresh();
    } else {
      onToast(r.error || "Remove failed", "error");
    }
  };

  const handleRefreshOAuth = async (prov: string, cid: string) => {
    const k = `${prov}:${cid}`;
    setRefreshingKey(k);
    const r = await apiPost(
      `/api/oauth/${encodeURIComponent(prov)}/${encodeURIComponent(cid)}/refresh`,
      {}
    );
    setRefreshingKey(null);
    if (r.ok) {
      onToast(`Refreshed OAuth token for ${prov}`, "success");
      onRefresh();
    } else {
      onToast(r.error || "Token refresh failed", "error");
    }
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="Providers & Accounts"
        description="Connect OAuth subscriptions (Antigravity, Claude, Codex, Copilot, Gemini) or API keys across 394 providers."
        badge={<Badge variant="accent">{total} catalog</Badge>}
        actions={
          <Button variant="primary" size="sm" onClick={() => onOpenAddCred("")}>
            <Plus className="h-3.5 w-3.5" />
            Add API Key
          </Button>
        }
      />

      {/* Section 1: Connected Accounts */}
      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <CheckCircle2 className="h-4 w-4 text-success" />
            <CardTitle>Connected Accounts</CardTitle>
            <Badge variant={credentials.length ? "success" : "default"}>
              {credentials.length}
            </Badge>
          </div>
        </CardHeader>
        <CardContent>
          {credentials.length === 0 ? (
            <EmptyState
              icon={<Plug className="h-5 w-5" />}
              title="No provider accounts connected yet"
              description="Click Login OAuth on Antigravity, Claude, Codex, or Copilot below — or add any provider API key."
            />
          ) : (
            <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
              {credentials.map((c) => {
                const k = `${c.provider}:${c.connection_id}`;
                const projectId =
                  c.provider_specific_json?.projectId || c.provider_specific_json?.project_id;
                return (
                  <div
                    key={k}
                    className="flex flex-col justify-between gap-3 rounded-xl border border-border bg-surface-subtle/50 p-4"
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="flex items-center gap-3 min-w-0">
                        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-border bg-surface">
                          <ProviderIcon providerId={c.provider} size={22} isDark={isDark} />
                        </div>
                        <div className="min-w-0">
                          <div className="truncate text-sm font-semibold text-fg">
                            {c.label || c.provider}
                          </div>
                          <div className="truncate font-mono text-[11px] text-fg-muted">
                            {c.provider} · {c.connection_id}
                          </div>
                        </div>
                      </div>
                      <Badge variant={c.health_status === "healthy" ? "success" : "danger"}>
                        {c.health_status || "healthy"}
                      </Badge>
                    </div>

                    <div className="flex flex-wrap items-center gap-1.5">
                      <Badge variant={c.auth_type === "oauth" ? "info" : "default"}>
                        {c.auth_type}
                      </Badge>
                      {c.expires_at && (
                        <Badge variant="outline" className="font-mono">
                          exp {fmtRelativeExpiry(c.expires_at)}
                        </Badge>
                      )}
                      {projectId && (
                        <Badge variant="accent" className="font-mono">
                          {String(projectId)}
                        </Badge>
                      )}
                    </div>

                    <div className="flex items-center justify-between border-t border-border pt-2.5">
                      <span className="truncate font-mono text-[11px] text-fg-muted">
                        {c.api_key_preview || "••••"}
                      </span>
                      <div className="flex items-center gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() =>
                            setInspectProvider({
                              id: c.provider,
                              name: c.label || c.provider,
                              connectionId: c.connection_id,
                            })
                          }
                          title="View available models"
                        >
                          <ListTree className="h-3.5 w-3.5 text-accent-ink" />
                          Models
                        </Button>
                        {c.auth_type === "oauth" && (
                          <Button
                            variant="ghost"
                            size="sm"
                            disabled={refreshingKey === k}
                            onClick={() => handleRefreshOAuth(c.provider, c.connection_id)}
                            title="Refresh OAuth token"
                          >
                            <RefreshCw
                              className={cn(
                                "h-3.5 w-3.5",
                                refreshingKey === k && "animate-spin"
                              )}
                            />
                            Refresh
                          </Button>
                        )}
                        <Button
                          variant="ghost"
                          size="icon"
                          className="text-danger hover:bg-danger-bg hover:text-danger"
                          onClick={() => handleDelete(c.provider, c.connection_id)}
                          title="Disconnect account"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>

      {/* Section 2: OAuth Subscription Providers (1:1 Upstream Featured Grid) */}
      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <KeyRound className="h-4 w-4 text-accent-ink" />
            <CardTitle>OAuth Subscriptions &amp; Coding Agents</CardTitle>
            <Badge variant="accent">{oauthProviders.length} ready</Badge>
          </div>
          <span className="text-xs text-fg-muted">Redirects directly to provider login page</span>
        </CardHeader>
        <CardContent>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
            {oauthProviders.map((p) => {
              const count = credentials.filter((c) => c.provider === p.id).length;
              return (
                <div
                  key={p.id}
                  className="flex flex-col justify-between gap-3 rounded-xl border border-border bg-surface-subtle/40 p-3.5 transition-all hover:border-border-strong hover:bg-surface-subtle"
                >
                  <div className="flex items-center gap-3 min-w-0">
                    <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-border bg-surface">
                      <ProviderIcon providerId={p.id} size={22} isDark={isDark} />
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-sm font-semibold text-fg">
                        {p.display_name}
                      </div>
                      <div className="truncate font-mono text-[11px] text-fg-muted">{p.id}</div>
                    </div>
                    {count > 0 && <Badge variant="success">{count} active</Badge>}
                  </div>
                  <div className="flex items-center justify-between gap-2">
                    <Badge
                      variant={
                        p.flow === "pkce"
                          ? "info"
                          : p.flow === "device_code"
                            ? "accent"
                            : "default"
                      }
                    >
                      {p.flow === "pkce"
                        ? "OAuth Login"
                        : p.flow === "device_code"
                          ? "Device Code"
                          : "Import"}
                    </Badge>
                    <div className="flex items-center gap-1.5">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() =>
                          setInspectProvider({ id: p.id, name: p.display_name })
                        }
                        title={`View models for ${p.display_name}`}
                      >
                        <ListTree className="h-3.5 w-3.5" />
                        Models
                      </Button>
                      <Button
                        variant="primary"
                        size="sm"
                        onClick={() => onLaunchOAuth(p.id, p.display_name)}
                      >
                        {p.flow === "import_only" ? "Import" : "Login OAuth"}
                      </Button>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        </CardContent>
      </Card>

      {/* Section 3: Full 394-Provider Directory */}
      <Card>
        <CardHeader className="flex-col items-stretch gap-3 sm:flex-row sm:items-center">
          <div className="flex items-center gap-2">
            <Sparkles className="h-4 w-4 text-accent-ink" />
            <CardTitle>All Providers Directory</CardTitle>
            <Badge variant="default">{total} providers</Badge>
          </div>
          <div className="relative w-full sm:w-72">
            <Search className="pointer-events-none absolute top-1/2 left-3 h-3.5 w-3.5 -translate-y-1/2 text-fg-muted" />
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search openai, deepseek, groq, free..."
              className="h-8 pl-9 text-xs"
            />
          </div>
        </CardHeader>

        <CardContent className="space-y-4">
          {/* Filter Chips */}
          <div className="flex flex-wrap gap-1.5">
            {(
              [
                ["all", "All"],
                ["oauth", "OAuth"],
                ["free", "Free Tier (82)"],
                ["openai", "OpenAI Format"],
                ["claude", "Claude Format"],
                ["gemini", "Gemini Format"],
              ] as Array<[FilterChip, string]>
            ).map(([val, label]) => (
              <button
                key={val}
                type="button"
                onClick={() => setFilter(val)}
                className={cn(
                  "rounded-full border px-3 py-1 text-xs font-medium transition-colors cursor-pointer",
                  filter === val
                    ? "border-accent bg-accent-subtle text-accent-ink"
                    : "border-border bg-surface-subtle/60 text-fg-muted hover:border-border-strong hover:text-fg"
                )}
              >
                {label}
              </button>
            ))}
          </div>

          {filteredProviders.length === 0 ? (
            <EmptyState
              title="No matching providers"
              description="Try another search term or switch the category filter."
            />
          ) : (
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
              {filteredProviders.map((p) => {
                const oauthTarget = oauthMap[p.id]
                  ? p.id
                  : OAUTH_ALIASES[p.id] && oauthMap[OAUTH_ALIASES[p.id]]
                    ? OAUTH_ALIASES[p.id]
                    : null;
                return (
                  <div
                    key={p.id}
                    className="flex flex-col justify-between gap-3 rounded-xl border border-border bg-surface-subtle/35 p-3.5 transition-all hover:border-border-strong hover:bg-surface-subtle/70"
                  >
                    <div className="flex items-center gap-3 min-w-0">
                      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-border bg-surface">
                        <ProviderIcon providerId={p.id} size={20} isDark={isDark} />
                      </div>
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-xs font-semibold text-fg">
                          {p.name || p.id}
                        </div>
                        <div className="truncate font-mono text-[10px] text-fg-muted">{p.id}</div>
                      </div>
                      {p.configured_accounts > 0 && (
                        <Badge variant="success">{p.configured_accounts}</Badge>
                      )}
                    </div>

                    <div className="flex items-center justify-between gap-2">
                      <div className="flex flex-wrap items-center gap-1">
                        <Badge variant="default">{p.format}</Badge>
                        <button
                          type="button"
                          onClick={() =>
                            setInspectProvider({ id: p.id, name: p.name || p.id })
                          }
                          title={`Inspect ${p.model_count} models for ${p.name || p.id}`}
                          className="inline-flex items-center gap-1 rounded-md border border-border-strong bg-surface px-2 py-0.5 font-mono text-[11px] font-medium text-fg hover:border-accent hover:text-accent-ink transition-colors cursor-pointer"
                        >
                          <ListTree className="h-3 w-3" />
                          {p.model_count} models
                        </button>
                        {p.is_free_tier && <Badge variant="accent">free</Badge>}
                      </div>
                      {oauthTarget ? (
                        <Button
                          variant="primary"
                          size="sm"
                          onClick={() => onLaunchOAuth(oauthTarget, p.name || p.id)}
                        >
                          Login OAuth
                        </Button>
                      ) : (
                        <Button
                          variant="default"
                          size="sm"
                          onClick={() => onOpenAddCred(p.id)}
                        >
                          Connect
                        </Button>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>

      <ProviderModelsModal
        provider={inspectProvider}
        credentials={credentials}
        isDark={isDark}
        onClose={() => setInspectProvider(null)}
        onSynced={onRefresh}
        onToast={onToast}
      />
    </div>
  );
}

interface ProviderModelItem {
  id: string;
  rawId: string;
  name: string;
  contextLength?: number | null;
  maxOutput?: number | null;
  reasoning?: boolean;
  vision?: boolean;
  free?: boolean;
  synced?: boolean;
}

function fmtCtx(n?: number | null): string | null {
  if (!n || n <= 0) return null;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n % 1_000_000 === 0 ? 0 : 1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`;
  return String(n);
}

function ProviderModelsModal({
  provider,
  credentials,
  isDark,
  onClose,
  onSynced,
  onToast,
}: {
  provider: { id: string; name: string; connectionId?: string } | null;
  credentials: CredentialItem[];
  isDark: boolean;
  onClose: () => void;
  onSynced: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [models, setModels] = useState<ProviderModelItem[]>([]);
  const [meta, setMeta] = useState<{ format?: string; baseUrl?: string; passthrough?: boolean }>({});
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const connectedCred = useMemo(() => {
    if (!provider) return null;
    return (
      credentials.find(
        (c) =>
          c.provider === provider.id &&
          (!provider.connectionId || c.connection_id === provider.connectionId)
      ) || null
    );
  }, [provider, credentials]);

  const loadModels = async (provId: string) => {
    setLoading(true);
    const [mRes, pRes] = await Promise.all([
      api<any>(`/v1/models?provider=${encodeURIComponent(provId)}&limit=500`),
      api<any>(`/api/providers/${encodeURIComponent(provId)}`),
    ]);
    setLoading(false);

    const byFullId = new Map<string, ProviderModelItem>();

    // 1. Live synced models saved on connected credentials
    for (const c of credentials.filter((x) => x.provider === provId)) {
      const cModels: string[] = Array.isArray((c as any).models) ? (c as any).models : [];
      for (const raw of cModels) {
        if (!raw || raw === "*") continue;
        const full = raw.includes("/") ? raw : `${provId}/${raw}`;
        byFullId.set(full, {
          id: full,
          rawId: raw,
          name: raw,
          synced: true,
        });
      }
    }

    // 2. Catalog models from /v1/models?provider=...
    if (mRes.ok && Array.isArray(mRes.data?.data)) {
      for (const item of mRes.data.data) {
        const full = String(item.id || "");
        if (!full) continue;
        const raw = full.startsWith(`${provId}/`) ? full.slice(provId.length + 1) : full;
        const prev = byFullId.get(full);
        byFullId.set(full, {
          id: full,
          rawId: raw,
          name: item.name || raw,
          contextLength: item.context_length,
          maxOutput: item.max_output_tokens,
          reasoning: !!item.supports_reasoning,
          vision: !!item.supports_vision,
          free: !!item.free_tier,
          synced: prev?.synced || false,
        });
      }
    }

    // 3. Fallback simple string models from /api/providers/{id}
    if (pRes.ok && pRes.data) {
      setMeta({
        format: pRes.data.format,
        baseUrl: Array.isArray(pRes.data.base_urls) ? pRes.data.base_urls[0] : undefined,
        passthrough: !!pRes.data.passthrough_models,
      });
      const rawList: string[] = Array.isArray(pRes.data.models) ? pRes.data.models : [];
      for (const raw of rawList) {
        if (!raw || raw === "*") continue;
        const full = `${provId}/${raw}`;
        if (!byFullId.has(full)) {
          byFullId.set(full, { id: full, rawId: raw, name: raw });
        }
      }
    }

    const sorted = Array.from(byFullId.values()).sort((a, b) => {
      if (a.synced !== b.synced) return a.synced ? -1 : 1;
      return a.rawId.localeCompare(b.rawId);
    });
    setModels(sorted);
  };

  useEffect(() => {
    if (provider) {
      setSearch("");
      setModels([]);
      setMeta({});
      void loadModels(provider.id);
    }
  }, [provider?.id, provider?.connectionId]);

  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return models;
    return models.filter(
      (m) =>
        m.id.toLowerCase().includes(q) ||
        m.name.toLowerCase().includes(q) ||
        m.rawId.toLowerCase().includes(q)
    );
  }, [models, search]);

  const handleCopy = async (id: string) => {
    const ok = await copyToClipboard(id);
    if (ok) {
      setCopiedId(id);
      setTimeout(() => setCopiedId((prev) => (prev === id ? null : prev)), 1500);
      onToast(`Copied "${id}"`, "info");
    } else {
      onToast("Copy failed", "error");
    }
  };

  const handleSyncLive = async () => {
    if (!provider || !connectedCred) return;
    setSyncing(true);
    const r = await apiPost(
      `/api/providers/${encodeURIComponent(provider.id)}/${encodeURIComponent(connectedCred.connection_id)}/sync-models`,
      {}
    );
    setSyncing(false);
    if (r.ok) {
      onToast(`Synced ${r.data?.models_count || 0} live models from ${provider.id}`, "success");
      onSynced();
      void loadModels(provider.id);
    } else {
      onToast(r.error || "Live model sync failed", "error");
    }
  };

  return (
    <Dialog
      open={!!provider}
      onClose={onClose}
      title={provider ? `${provider.name} — Models` : "Models"}
      description={
        provider
          ? `${provider.id}${meta.format ? ` · ${meta.format}` : ""}${meta.baseUrl ? ` · ${meta.baseUrl}` : ""}`
          : undefined
      }
      icon={provider ? <ProviderIcon providerId={provider.id} size={24} isDark={isDark} /> : undefined}
      maxWidth="max-w-2xl"
    >
      <div className="space-y-3">
        <div className="flex flex-wrap items-center gap-2">
          <div className="relative flex-1 min-w-[200px]">
            <Search className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-fg-muted" />
            <Input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Filter models (e.g. claude, gemini, gpt, flash)..."
              className="pl-8 text-xs"
            />
          </div>
          <Badge variant="accent" className="font-mono">
            {filtered.length} / {models.length}
          </Badge>
          {connectedCred && (
            <Button
              variant="default"
              size="sm"
              disabled={syncing}
              onClick={() => void handleSyncLive()}
            >
              <RefreshCw className={cn("h-3.5 w-3.5", syncing && "animate-spin")} />
              {syncing ? "Syncing..." : "Sync Live"}
            </Button>
          )}
        </div>

        {loading ? (
          <div className="py-12 text-center text-xs text-fg-muted">Loading models...</div>
        ) : filtered.length === 0 ? (
          <EmptyState
            title={
              meta.passthrough
                ? "Passthrough / Dynamic Provider"
                : "No models found for this provider"
            }
            description={
              meta.passthrough
                ? `This provider accepts any upstream model ID directly as "${provider?.id}/<model-id>"${connectedCred ? " — or click Sync Live above to fetch available models." : "."}`
                : "Try clearing the filter or connecting an account and clicking Sync Live."
            }
          />
        ) : (
          <div className="max-h-[55vh] overflow-y-auto divide-y divide-border rounded-xl border border-border">
            {filtered.map((m) => {
              const ctx = fmtCtx(m.contextLength);
              return (
                <div
                  key={m.id}
                  className="flex items-center justify-between gap-3 px-3.5 py-2.5 hover:bg-surface-subtle/50"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <code className="truncate font-mono text-xs font-semibold text-fg">
                        {m.id}
                      </code>
                      {m.synced && <Badge variant="success">live</Badge>}
                      {m.free && <Badge variant="accent">free</Badge>}
                      {ctx && (
                        <Badge variant="outline" className="font-mono">
                          {ctx} ctx
                        </Badge>
                      )}
                      {m.reasoning && (
                        <Badge variant="info">
                          <Brain className="h-3 w-3" /> reasoning
                        </Badge>
                      )}
                      {m.vision && (
                        <Badge variant="default">
                          <Eye className="h-3 w-3" /> vision
                        </Badge>
                      )}
                    </div>
                    {m.name && m.name !== m.rawId && (
                      <div className="truncate text-[11px] text-fg-muted mt-0.5">{m.name}</div>
                    )}
                  </div>
                  <Button
                    variant="default"
                    size="sm"
                    onClick={() => void handleCopy(m.id)}
                    className="shrink-0"
                  >
                    {copiedId === m.id ? (
                      <Check className="h-3.5 w-3.5 text-success" />
                    ) : (
                      <Copy className="h-3.5 w-3.5" />
                    )}
                    Copy ID
                  </Button>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </Dialog>
  );
}
