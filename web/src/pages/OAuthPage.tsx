import { useState } from "react";
import {
  KeyRound,
  RefreshCw,
  Upload,
  CheckCircle2,
  Trash2,
} from "lucide-react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Badge,
  Button,
  Input,
  PageHeader,
  EmptyState,
} from "@/components/ui/primitives";
import { ProviderIcon } from "@/components/ProviderIcon";
import {
  apiDelete,
  apiPost,
  type CredentialItem,
  type GatewayHealth,
  type OAuthProviderMeta,
} from "@/lib/api";
import { fmtRelativeExpiry } from "@/lib/utils";

export function OAuthPage({
  oauthProviders,
  oauthStatus,
  credentials,
  isDark,
  onLaunchOAuth,
  onRefresh,
  onToast,
}: {
  oauthProviders: OAuthProviderMeta[];
  oauthStatus: GatewayHealth["oauth"] | undefined;
  credentials: CredentialItem[];
  isDark: boolean;
  onLaunchOAuth: (providerId: string, displayName: string) => void;
  onRefresh: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [sweeping, setSweeping] = useState(false);
  const [impProv, setImpProv] = useState("claude");
  const [impCid, setImpCid] = useState("");
  const [impAt, setImpAt] = useState("");
  const [impRt, setImpRt] = useState("");

  const oauthCreds = credentials.filter((c) => c.auth_type === "oauth");

  const runSweep = async () => {
    setSweeping(true);
    const r = await apiPost("/api/oauth/sweep", {});
    setSweeping(false);
    if (r.ok) {
      onToast(
        `Refresh sweep complete: refreshed=${r.data?.refreshed ?? 0}, needs_reauth=${r.data?.needs_reauth ?? 0}`,
        "success"
      );
      onRefresh();
    } else {
      onToast(r.error || "Sweep failed", "error");
    }
  };

  const manualImport = async () => {
    if (!impProv.trim() || !impAt.trim()) {
      onToast("Provider and access token are required", "error");
      return;
    }
    const r = await apiPost(`/api/oauth/${encodeURIComponent(impProv.trim())}/import`, {
      connection_id: impCid.trim() || undefined,
      access_token: impAt.trim(),
      refresh_token: impRt.trim() || undefined,
    });
    if (r.ok) {
      setImpAt("");
      setImpRt("");
      onToast(`Imported OAuth token for ${impProv.trim()}`, "success");
      onRefresh();
    } else {
      onToast(r.error || "Import failed", "error");
    }
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="OAuth 2.0 & Token Refresh"
        description="PKCE S256 browser login, RFC 8628 device-code polling, Antigravity Cloud Code project discovery & proactive background token refresh."
        actions={
          <Button variant="primary" size="sm" disabled={sweeping} onClick={runSweep}>
            <RefreshCw className={`h-3.5 w-3.5 ${sweeping ? "animate-spin" : ""}`} />
            Run Refresh Sweep
          </Button>
        }
      />

      {/* 3 Status Cards */}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <Card className="p-4">
          <div className="text-xs text-fg-muted">Supported OAuth Providers</div>
          <div className="mt-1.5 text-2xl font-semibold text-fg">{oauthProviders.length}</div>
          <div className="mt-1 text-xs text-fg-muted">PKCE S256 &amp; RFC 8628 Device Code</div>
        </Card>
        <Card className="p-4">
          <div className="text-xs text-fg-muted">Connected OAuth Accounts</div>
          <div className="mt-1.5 text-2xl font-semibold text-fg">
            {oauthStatus?.connected_oauth_accounts ?? oauthCreds.length}
          </div>
          <div className="mt-1 text-xs text-fg-muted">
            Pending sessions: {oauthStatus?.active_pending_sessions ?? 0}
          </div>
        </Card>
        <Card className="p-4">
          <div className="text-xs text-fg-muted">Proactive Refresh Scheduler</div>
          <div className="mt-1.5 flex items-center gap-2">
            <span className="text-2xl font-semibold text-fg">
              {oauthStatus?.healthcheck_disabled ? "OFF" : "ACTIVE"}
            </span>
            <Badge variant={oauthStatus?.healthcheck_disabled ? "warning" : "success"}>
              {oauthStatus?.healthcheck_disabled ? "Paused" : "60s tick"}
            </Badge>
          </div>
          <div className="mt-1 text-xs text-fg-muted">
            Google slot spacing: {oauthStatus?.google_refresh_delay_ms ?? 0}ms
          </div>
        </Card>
      </div>

      {/* Connected OAuth Accounts */}
      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <CheckCircle2 className="h-4 w-4 text-success" />
            <CardTitle>Active OAuth Sessions</CardTitle>
          </div>
          <Badge variant="success">{oauthCreds.length} connected</Badge>
        </CardHeader>
        <CardContent>
          {oauthCreds.length === 0 ? (
            <EmptyState
              title="No OAuth accounts connected"
              description="Click Login OAuth on any provider card below to authenticate directly on its official login page."
            />
          ) : (
            <div className="overflow-x-auto rounded-lg border border-border">
              <table className="w-full text-left text-xs">
                <thead className="bg-surface-subtle text-fg-muted">
                  <tr>
                    <th className="px-3.5 py-2.5 font-medium">Account</th>
                    <th className="px-3.5 py-2.5 font-medium">Connection ID</th>
                    <th className="px-3.5 py-2.5 font-medium">Expiry</th>
                    <th className="px-3.5 py-2.5 font-medium">Health</th>
                    <th className="px-3.5 py-2.5 text-right font-medium">Actions</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-border">
                  {oauthCreds.map((c) => (
                    <tr key={`${c.provider}:${c.connection_id}`}>
                      <td className="px-3.5 py-2.5">
                        <div className="flex items-center gap-2.5">
                          <ProviderIcon providerId={c.provider} size={18} isDark={isDark} />
                          <div>
                            <div className="font-semibold text-fg">{c.label || c.provider}</div>
                            <div className="font-mono text-[10px] text-fg-muted">{c.provider}</div>
                          </div>
                        </div>
                      </td>
                      <td className="px-3.5 py-2.5 font-mono text-fg-muted">{c.connection_id}</td>
                      <td className="px-3.5 py-2.5 font-mono text-fg-muted">
                        {fmtRelativeExpiry(c.expires_at)}
                      </td>
                      <td className="px-3.5 py-2.5">
                        <Badge variant={c.health_status === "healthy" ? "success" : "danger"}>
                          {c.health_status || "healthy"}
                        </Badge>
                      </td>
                      <td className="px-3.5 py-2.5 text-right">
                        <div className="inline-flex items-center gap-1">
                          <Button
                            variant="default"
                            size="sm"
                            onClick={async () => {
                              const r = await apiPost(
                                `/api/oauth/${encodeURIComponent(c.provider)}/${encodeURIComponent(c.connection_id)}/refresh`,
                                {}
                              );
                              if (r.ok) {
                                onToast(`Refreshed ${c.provider}`, "success");
                                onRefresh();
                              } else {
                                onToast(r.error || "Refresh failed", "error");
                              }
                            }}
                          >
                            <RefreshCw className="h-3 w-3" />
                            Refresh
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="text-danger hover:bg-danger-bg"
                            onClick={async () => {
                              await apiDelete(
                                `/api/providers/credentials/${encodeURIComponent(c.provider)}/${encodeURIComponent(c.connection_id)}`
                              );
                              onToast("Disconnected account", "info");
                              onRefresh();
                            }}
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </Button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>

      {/* OAuth Providers Grid + Manual Token Import */}
      <div className="grid grid-cols-1 gap-5 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader>
            <div className="flex items-center gap-2">
              <KeyRound className="h-4 w-4 text-accent-ink" />
              <CardTitle>All Supported OAuth Providers</CardTitle>
            </div>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              {oauthProviders.map((p) => (
                <div
                  key={p.id}
                  className="flex items-center justify-between gap-3 rounded-xl border border-border bg-surface-subtle/40 p-3 hover:border-border-strong"
                >
                  <div className="flex items-center gap-3 min-w-0">
                    <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-border bg-surface">
                      <ProviderIcon providerId={p.id} size={20} isDark={isDark} />
                    </div>
                    <div className="min-w-0">
                      <div className="truncate text-xs font-semibold text-fg">
                        {p.display_name}
                      </div>
                      <div className="flex items-center gap-1.5 mt-0.5">
                        <span className="font-mono text-[10px] text-fg-muted">{p.id}</span>
                        <Badge
                          variant={
                            p.flow === "pkce"
                              ? "info"
                              : p.flow === "device_code"
                                ? "accent"
                                : "default"
                          }
                        >
                          {p.flow}
                        </Badge>
                      </div>
                    </div>
                  </div>
                  <Button
                    variant="primary"
                    size="sm"
                    onClick={() => onLaunchOAuth(p.id, p.display_name)}
                  >
                    Login
                  </Button>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <Upload className="h-4 w-4 text-accent-ink" />
              <CardTitle>Manual Token Import</CardTitle>
            </div>
          </CardHeader>
          <CardContent className="space-y-3">
            <p className="text-xs text-fg-muted">
              Directly save an existing OAuth access token, Claude setup-token, or IDE token (Cursor / Trae).
            </p>
            <div className="space-y-1">
              <label className="text-xs font-medium text-fg">Provider ID</label>
              <Input
                value={impProv}
                onChange={(e) => setImpProv(e.target.value)}
                placeholder="claude, antigravity, codex, cursor..."
              />
            </div>
            <div className="space-y-1">
              <label className="text-xs font-medium text-fg">Connection ID (optional)</label>
              <Input
                value={impCid}
                onChange={(e) => setImpCid(e.target.value)}
                placeholder="work-account"
              />
            </div>
            <div className="space-y-1">
              <label className="text-xs font-medium text-fg">Access Token</label>
              <Input
                type="password"
                value={impAt}
                onChange={(e) => setImpAt(e.target.value)}
                placeholder="Paste access token..."
              />
            </div>
            <div className="space-y-1">
              <label className="text-xs font-medium text-fg">Refresh Token (optional)</label>
              <Input
                type="password"
                value={impRt}
                onChange={(e) => setImpRt(e.target.value)}
                placeholder="Paste refresh token..."
              />
            </div>
            <Button variant="primary" className="w-full" onClick={manualImport}>
              Import Token
            </Button>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
