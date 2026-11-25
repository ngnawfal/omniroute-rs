import { useCallback, useEffect, useState } from "react";
import {
  Gauge,
  Globe,
  RefreshCw,
  CheckCircle2,
  XCircle,
  Plus,
  Trash2,
  Zap,
  Sparkles,
} from "lucide-react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Badge,
  Button,
  Input,
  Switch,
  PageHeader,
  EmptyState,
} from "@/components/ui/primitives";
import { ProviderIcon } from "@/components/ProviderIcon";
import { api, apiPost, apiPut } from "@/lib/api";

export function QuotaProxyPage({
  isDark,
  onToast,
}: {
  isDark: boolean;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [accounts, setAccounts] = useState<any[]>([]);
  const [loadingQuota, setLoadingQuota] = useState(false);
  const [syncingKey, setSyncingKey] = useState<string | null>(null);

  // Proxy state
  const [globalProxy, setGlobalProxy] = useState("");
  const [applyToOAuth, setApplyToOAuth] = useState(false);
  const [poolText, setPoolText] = useState("");
  const [providerProxies, setProviderProxies] = useState<Array<{ provider: string; url: string }>>(
    []
  );
  const [newProvName, setNewProvName] = useState("");
  const [newProvUrl, setNewProvUrl] = useState("");
  const [accountOverrides, setAccountOverrides] = useState<any[]>([]);
  const [cachedClients, setCachedClients] = useState(0);

  // Live proxy test state
  const [testUrl, setTestUrl] = useState("");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<any>(null);

  const loadQuota = useCallback(async (refresh = false) => {
    setLoadingQuota(true);
    const r = await api<any>(`/api/quota${refresh ? "?refresh=true" : ""}`);
    setLoadingQuota(false);
    if (r.ok) setAccounts(r.data?.accounts || []);
  }, []);

  const loadProxies = useCallback(async () => {
    const r = await api<any>("/api/proxies");
    if (r.ok) {
      const cfg = r.data?.config || {};
      setGlobalProxy(cfg.global || "");
      setApplyToOAuth(!!cfg.apply_to_oauth);
      setPoolText((cfg.pool || []).join("\n"));
      const map = cfg.providers || {};
      setProviderProxies(Object.entries(map).map(([provider, url]) => ({ provider, url: String(url) })));
      setAccountOverrides(r.data?.account_overrides || []);
      setCachedClients(r.data?.cached_clients || 0);
    }
  }, []);

  useEffect(() => {
    void loadQuota(false);
    void loadProxies();
  }, [loadQuota, loadProxies]);

  const syncModels = async (provider: string, connectionId: string) => {
    const key = `${provider}::${connectionId}`;
    setSyncingKey(key);
    const r = await apiPost(
      `/api/providers/${encodeURIComponent(provider)}/${encodeURIComponent(connectionId)}/sync-models`,
      {}
    );
    setSyncingKey(null);
    if (r.ok) {
      onToast(`Synced ${r.data?.models_count || 0} live models for ${provider}/${connectionId}`, "success");
      void loadQuota(false);
    } else {
      onToast(r.error || "Live model sync failed", "error");
    }
  };

  const saveProxies = async () => {
    const providersObj: Record<string, string> = {};
    for (const row of providerProxies) {
      if (row.provider.trim() && row.url.trim()) {
        providersObj[row.provider.trim()] = row.url.trim();
      }
    }
    const pool = poolText
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean);
    const r = await apiPut("/api/proxies", {
      global: globalProxy.trim() || null,
      providers: providersObj,
      pool,
      apply_to_oauth: applyToOAuth,
    });
    if (r.ok) {
      onToast("Upstream proxy configuration saved", "success");
      void loadProxies();
      void loadQuota(false);
    } else {
      onToast(r.error || "Invalid proxy URL", "error");
    }
  };

  const runProxyTest = async (urlOverride?: string) => {
    const target = (urlOverride ?? testUrl).trim() || globalProxy.trim();
    if (!target) {
      onToast("Enter a proxy URL (http:// or socks5://) to test", "error");
      return;
    }
    setTestUrl(target);
    setTesting(true);
    setTestResult(null);
    const r = await apiPost("/api/proxies/test", { proxy_url: target });
    setTesting(false);
    if (r.ok) {
      setTestResult(r.data);
      onToast(
        r.data?.ok
          ? `Proxy OK · ${r.data.latency_ms}ms · Exit IP ${r.data.exit_ip || "verified"}`
          : `Proxy test failed: ${r.data?.error || `HTTP ${r.data?.status}`}`,
        r.data?.ok ? "success" : "error"
      );
    } else {
      onToast(r.error || "Proxy test failed", "error");
    }
  };

  const addProviderOverride = () => {
    if (!newProvName.trim() || !newProvUrl.trim()) return;
    setProviderProxies((prev) => [
      ...prev.filter((p) => p.provider !== newProvName.trim()),
      { provider: newProvName.trim(), url: newProvUrl.trim() },
    ]);
    setNewProvName("");
    setNewProvUrl("");
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="Live Quota Monitor & Upstream Proxies"
        description="Track Antigravity per-family quota (Gemini/Claude/Argon), auto-sync live OAuth models & route accounts via HTTP/SOCKS5 proxies."
        actions={
          <Button variant="primary" size="sm" onClick={() => void loadQuota(true)} disabled={loadingQuota}>
            <RefreshCw className={`h-3.5 w-3.5 ${loadingQuota ? "animate-spin" : ""}`} />
            Probe Live Quota
          </Button>
        }
      />

      {/* Live Quota & Synced Models */}
      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <Gauge className="h-4 w-4 text-accent-ink" />
            <div>
              <CardTitle>Connected Account Quota &amp; Live Model Sync</CardTitle>
              <CardDescription>
                Shows live remaining quota fractions, Antigravity family lockouts, token expiry &amp; synced models
              </CardDescription>
            </div>
          </div>
          <Badge variant="default">{accounts.length} account(s)</Badge>
        </CardHeader>
        <CardContent>
          {accounts.length === 0 ? (
            <EmptyState
              title="No provider credentials connected"
              description="Connect an OAuth account (Antigravity, Claude, Codex, Copilot) or API key to inspect live quota and sync models."
            />
          ) : (
            <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
              {accounts.map((acc) => {
                const key = `${acc.provider}::${acc.connection_id}`;
                const expMins =
                  typeof acc.expires_in_ms === "number"
                    ? Math.round(acc.expires_in_ms / 60000)
                    : null;
                return (
                  <div
                    key={key}
                    className="rounded-xl border border-border bg-surface-subtle/35 p-4 space-y-3"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="flex items-center gap-2.5 min-w-0">
                        <ProviderIcon providerId={acc.provider} size={24} isDark={isDark} />
                        <div className="min-w-0">
                          <div className="flex items-center gap-1.5">
                            <span className="truncate text-sm font-semibold text-fg">
                              {acc.label || acc.connection_id}
                            </span>
                            <Badge
                              variant={acc.health_status === "healthy" ? "success" : "danger"}
                            >
                              {acc.health_status}
                            </Badge>
                          </div>
                          <div className="font-mono text-[11px] text-fg-muted">
                            {acc.provider} / {acc.connection_id} · {acc.auth_type}
                          </div>
                        </div>
                      </div>
                      <Button
                        variant="default"
                        size="sm"
                        disabled={syncingKey === key}
                        onClick={() => void syncModels(acc.provider, acc.connection_id)}
                      >
                        <Sparkles className="h-3.5 w-3.5 text-accent-ink" />
                        {syncingKey === key ? "Syncing..." : "Sync Models"}
                      </Button>
                    </div>

                    {/* Antigravity Per-Family Quota Bars */}
                    {Array.isArray(acc.families) && (
                      <div className="grid grid-cols-3 gap-2">
                        {acc.families.map((f: any) => {
                          const frac =
                            typeof f.remaining_fraction === "number"
                              ? Math.round(f.remaining_fraction * 100)
                              : null;
                          const lockedSec = Math.ceil((f.locked_remaining_ms || 0) / 1000);
                          return (
                            <div
                              key={f.family}
                              className="rounded-lg border border-border bg-surface px-2.5 py-2 space-y-1"
                            >
                              <div className="flex items-center justify-between text-[11px]">
                                <span className="font-semibold capitalize text-fg">{f.family}</span>
                                <Badge variant={lockedSec > 0 ? "warning" : "success"}>
                                  {lockedSec > 0 ? `${lockedSec}s` : frac !== null ? `${frac}%` : "OK"}
                                </Badge>
                              </div>
                              <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-subtle">
                                <div
                                  className={`h-full rounded-full ${lockedSec > 0 ? "bg-warning" : "bg-success"}`}
                                  style={{ width: `${lockedSec > 0 ? 15 : frac ?? 100}%` }}
                                />
                              </div>
                            </div>
                          );
                        })}
                      </div>
                    )}

                    <div className="flex flex-wrap items-center gap-2 text-[11px] text-fg-muted font-mono">
                      <span>Models synced: {acc.synced_models_count || 0}</span>
                      {expMins !== null && (
                        <span>
                          · Token {expMins > 0 ? `expires in ${expMins}m` : "expired"}
                        </span>
                      )}
                      {acc.proxy_url && (
                        <Badge variant="accent" className="font-mono">
                          Proxy: {acc.proxy_url}
                        </Badge>
                      )}
                    </div>

                    {acc.live_error && (
                      <div className="rounded-lg border border-danger/30 bg-danger/10 px-2.5 py-1.5 font-mono text-[11px] text-danger">
                        {acc.live_error}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>

      {/* HTTP / SOCKS5 Upstream Proxy Manager */}
      <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <Globe className="h-4 w-4 text-accent-ink" />
              <div>
                <CardTitle>HTTP / SOCKS5 Upstream Proxy &amp; Pool</CardTitle>
                <CardDescription>
                  Priority: Per-Account &gt; Per-Provider &gt; Global &gt; Pool Round-Robin &gt; Direct
                </CardDescription>
              </div>
            </div>
            <Badge variant="default">{cachedClients} pooled</Badge>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-1">
              <label className="text-xs font-medium text-fg">
                Global Fallback Proxy (optional — <code className="font-mono">http://</code>, <code className="font-mono">socks5h://user:pass@host:port</code>)
              </label>
              <Input
                value={globalProxy}
                onChange={(e) => setGlobalProxy(e.target.value)}
                placeholder="socks5h://127.0.0.1:1080 or http://proxy.lan:8080"
                className="font-mono text-xs"
              />
            </div>

            <div className="flex items-center justify-between rounded-xl border border-border bg-surface-subtle/35 px-3.5 py-2.5">
              <div>
                <div className="text-xs font-semibold text-fg">Apply Global Proxy to OAuth Refreshes</div>
                <div className="text-[11px] text-fg-muted">
                  Also route token exchange &amp; refresh calls via configured proxy
                </div>
              </div>
              <Switch checked={applyToOAuth} onChange={setApplyToOAuth} />
            </div>

            <div className="space-y-1">
              <label className="text-xs font-medium text-fg">
                Proxy Pool (one proxy URL per line — round-robin when Global Proxy is blank)
              </label>
              <textarea
                rows={3}
                value={poolText}
                onChange={(e) => setPoolText(e.target.value)}
                placeholder={"socks5h://10.0.0.2:1080\nhttp://10.0.0.3:8080"}
                className="w-full rounded-lg border border-border-strong bg-surface px-3 py-2 font-mono text-xs text-fg placeholder:text-fg-muted/60 focus:border-ring focus:outline-none"
              />
            </div>

            <div className="space-y-2">
              <label className="text-xs font-medium text-fg">
                Per-Provider Overrides (use <code className="font-mono">direct</code> to bypass global proxy)
              </label>
              <div className="flex gap-2">
                <Input
                  value={newProvName}
                  onChange={(e) => setNewProvName(e.target.value)}
                  placeholder="antigravity"
                  className="w-36 font-mono text-xs"
                />
                <Input
                  value={newProvUrl}
                  onChange={(e) => setNewProvUrl(e.target.value)}
                  placeholder="socks5h://... or direct"
                  className="flex-1 font-mono text-xs"
                />
                <Button variant="default" size="sm" onClick={addProviderOverride}>
                  <Plus className="h-3.5 w-3.5" />
                </Button>
              </div>

              {providerProxies.length > 0 && (
                <div className="space-y-1.5">
                  {providerProxies.map((row) => (
                    <div
                      key={row.provider}
                      className="flex items-center justify-between gap-2 rounded-lg border border-border bg-surface-subtle/40 px-3 py-1.5 text-xs"
                    >
                      <div className="flex items-center gap-2 min-w-0">
                        <ProviderIcon providerId={row.provider} size={16} isDark={isDark} />
                        <span className="font-semibold text-fg">{row.provider}</span>
                        <span className="truncate font-mono text-[11px] text-fg-muted">
                          {row.url}
                        </span>
                      </div>
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() =>
                          setProviderProxies((prev) =>
                            prev.filter((p) => p.provider !== row.provider)
                          )
                        }
                      >
                        <Trash2 className="h-3.5 w-3.5 text-danger" />
                      </Button>
                    </div>
                  ))}
                </div>
              )}
            </div>

            <div className="flex justify-end">
              <Button variant="primary" onClick={saveProxies}>
                Save Proxy Config
              </Button>
            </div>
          </CardContent>
        </Card>

        {/* Live Proxy Tester + Per-Account Overrides */}
        <div className="space-y-5">
          <Card>
            <CardHeader>
              <div className="flex items-center gap-2">
                <Zap className="h-4 w-4 text-accent-ink" />
                <div>
                  <CardTitle>Live Proxy Connectivity &amp; Exit-IP Test</CardTitle>
                  <CardDescription>
                    Verifies HTTP/HTTPS/SOCKS5 reachability, latency &amp; egress IP before saving
                  </CardDescription>
                </div>
              </div>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="flex gap-2">
                <Input
                  value={testUrl}
                  onChange={(e) => setTestUrl(e.target.value)}
                  placeholder="socks5h://user:pass@host:1080"
                  className="flex-1 font-mono text-xs"
                />
                <Button variant="primary" size="sm" disabled={testing} onClick={() => void runProxyTest()}>
                  {testing ? "Testing..." : "Test Proxy"}
                </Button>
              </div>
              {testResult && (
                <div
                  className={`rounded-xl border p-3 text-xs space-y-1 ${
                    testResult.ok
                      ? "border-success/40 bg-success/10 text-fg"
                      : "border-danger/40 bg-danger/10 text-danger"
                  }`}
                >
                  <div className="flex items-center gap-2 font-semibold">
                    {testResult.ok ? (
                      <CheckCircle2 className="h-4 w-4 text-success" />
                    ) : (
                      <XCircle className="h-4 w-4 text-danger" />
                    )}
                    <span>
                      {testResult.ok
                        ? `Connected in ${testResult.latency_ms}ms`
                        : `Failed (${testResult.latency_ms}ms)`}
                    </span>
                  </div>
                  {testResult.exit_ip && (
                    <div className="font-mono text-[11px]">Exit IP: {testResult.exit_ip}</div>
                  )}
                  {testResult.error && (
                    <div className="font-mono text-[11px] break-all">{testResult.error}</div>
                  )}
                </div>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Per-Account Proxy Overrides</CardTitle>
              <Badge variant="default">{accountOverrides.length}</Badge>
            </CardHeader>
            <CardContent>
              {accountOverrides.length === 0 ? (
                <EmptyState
                  title="No per-account proxy overrides"
                  description="Set an Upstream Proxy URL when adding/editing a credential to isolate each OAuth account on its own IP."
                />
              ) : (
                <div className="space-y-2">
                  {accountOverrides.map((o, idx) => (
                    <div
                      key={idx}
                      className="flex items-center justify-between gap-2 rounded-lg border border-border bg-surface-subtle/40 px-3 py-2 text-xs"
                    >
                      <div className="flex items-center gap-2 min-w-0">
                        <ProviderIcon providerId={o.provider} size={18} isDark={isDark} />
                        <div className="min-w-0">
                          <div className="font-medium text-fg">
                            {o.provider} / {o.connection_id}
                          </div>
                          <div className="truncate font-mono text-[11px] text-fg-muted">
                            {o.proxy_url}
                          </div>
                        </div>
                      </div>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => void runProxyTest(o.proxy_url)}
                      >
                        Test
                      </Button>
                    </div>
                  ))}
                </div>
              )}
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
