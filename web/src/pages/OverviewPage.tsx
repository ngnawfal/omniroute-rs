import { useState } from "react";
import {
  Layers,
  Activity,
  Sparkles,
  Trophy,
  Copy,
  Check,
  ShieldCheck,
  ArrowUpRight,
  Terminal,
  KeyRound,
  Plus,
} from "lucide-react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Badge,
  Button,
  PageHeader,
  EmptyState,
  SegmentedControl,
} from "@/components/ui/primitives";
import { ProviderIcon } from "@/components/ProviderIcon";
import { copyToClipboard, fmtCost, fmtNumber } from "@/lib/utils";
import {
  apiPost,
  getStoredApiKey,
  setStoredApiKey,
  type CredentialItem,
  type GamificationStatus,
  type GatewayHealth,
} from "@/lib/api";
import type { NavTab } from "@/components/Sidebar";

const QUICK_OAUTH = [
  { id: "antigravity", name: "Antigravity" },
  { id: "claude", name: "Claude Code" },
  { id: "codex", name: "OpenAI Codex" },
  { id: "github", name: "GitHub Copilot" },
  { id: "gemini-cli", name: "Gemini CLI" },
  { id: "kimi-coding", name: "Kimi Coding" },
];

export function OverviewPage({
  health,
  gamification,
  credentials,
  keys = [],
  isDark,
  onNavigate,
  onLaunchOAuth,
  onOpenAddCred,
  onOpenApiKeys,
  onRefresh,
  onToast,
}: {
  health: GatewayHealth | null;
  gamification: GamificationStatus | null;
  credentials: CredentialItem[];
  keys?: any[];
  isDark: boolean;
  onNavigate: (tab: NavTab) => void;
  onLaunchOAuth: (providerId: string, displayName: string) => void;
  onOpenAddCred: (providerId?: string) => void;
  onOpenApiKeys?: () => void;
  onRefresh?: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [sdkTab, setSdkTab] = useState<"openai" | "claude" | "curl" | "mcp">("openai");
  const [copied, setCopied] = useState(false);
  const [genKeyBusy, setGenKeyBusy] = useState(false);

  const origin = window.location.origin;
  const firstDbKey = keys.find((k: any) => !k.disabled && k.key)?.key || "";
  const apiKey = getStoredApiKey() || firstDbKey || "omniroute-local";
  const hasRealKey = !!(getStoredApiKey() || firstDbKey);

  const quickCreateKey = async () => {
    setGenKeyBusy(true);
    const r = await apiPost("/api/keys", { label: "quick-connect" });
    setGenKeyBusy(false);
    if (r.ok && r.data?.key) {
      const k = String(r.data.key);
      setStoredApiKey(k);
      const ok = await copyToClipboard(k);
      onToast(
        ok ? "API key generated, activated & copied!" : "API key generated & activated!",
        "success"
      );
      onRefresh?.();
    } else {
      onToast(r.error || "Failed to generate API key", "error");
    }
  };

  const snippets: Record<typeof sdkTab, string> = {
    openai: `from openai import OpenAI

client = OpenAI(
    base_url="${origin}/v1",
    api_key="${apiKey}",
)

resp = client.chat.completions.create(
    model="antigravity/claude-sonnet-4-6",
    messages=[{"role": "user", "content": "Hello from OmniRoute Rust!"}],
)
print(resp.choices[0].message.content)`,
    claude: `export ANTHROPIC_BASE_URL="${origin}"
export ANTHROPIC_API_KEY="${apiKey}"
claude`,
    curl: `curl ${origin}/v1/chat/completions \\
  -H "Authorization: Bearer ${apiKey}" \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "pollinations/openai-fast",
    "messages": [{"role": "user", "content": "Hi!"}]
  }'`,
    mcp: `curl -X POST ${origin}/api/mcp \\
  -H "Content-Type: application/json" \\
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'`,
  };

  const copySnippet = async () => {
    const ok = await copyToClipboard(snippets[sdkTab]);
    if (ok) {
      setCopied(true);
      onToast("Snippet copied to clipboard", "info");
      setTimeout(() => setCopied(false), 1800);
    } else {
      onToast("Copy failed", "error");
    }
  };

  const hitRate = Number(health?.cache?.stats?.hit_rate ?? 0);
  const compSaved = Number(health?.compression?.stats?.total_tokens_saved ?? 0);
  const cacheEntries = Number(health?.cache?.stats?.entries ?? 0);
  const xpPct = gamification
    ? Math.min(
        100,
        Math.max(
          8,
          Math.round(
            ((gamification.total_xp % Math.max(1, gamification.xp_to_next_level || 100)) /
              Math.max(1, gamification.xp_to_next_level || 100)) *
              100
          )
        )
      )
    : 8;

  const breakers = health?.breakers || [];
  const lockouts = health?.active_lockouts || [];

  return (
    <div className="space-y-6">
      <PageHeader
        title="Gateway Overview"
        description="Unified native Rust AI router — 394 providers, 3,239 models, OAuth 2.0, semantic cache & MITM."
        badge={
          <Badge variant="success" className="font-mono">
            v{health?.upstream_sync_version || "3.8.55"}
          </Badge>
        }
        actions={
          <>
            <Button variant="default" size="sm" onClick={() => onNavigate("playground")}>
              <ArrowUpRight className="h-3.5 w-3.5" />
              Open Playground
            </Button>
            <Button variant="primary" size="sm" onClick={() => onOpenAddCred("")}>
              <Plus className="h-3.5 w-3.5" />
              Add Credential
            </Button>
          </>
        }
      />

      {/* 4 KPI Stat Cards */}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <Card className="p-5">
          <div className="flex items-center justify-between text-xs font-medium text-fg-muted">
            <span>Catalog Coverage</span>
            <Layers className="h-4 w-4 text-accent-ink" />
          </div>
          <div className="mt-2.5 text-3xl font-semibold tracking-tight text-fg tabular-nums">
            {fmtNumber(health?.provider_count || 394)}
          </div>
          <div className="mt-2 flex items-center justify-between text-xs text-fg-muted">
            <span>{fmtNumber(health?.model_count || 3239)} models</span>
            <Badge variant="accent">{health?.routing_strategies || 22} strategies</Badge>
          </div>
        </Card>

        <Card className="p-5">
          <div className="flex items-center justify-between text-xs font-medium text-fg-muted">
            <span>Gateway Traffic</span>
            <Activity className="h-4 w-4 text-info" />
          </div>
          <div className="mt-2.5 text-3xl font-semibold tracking-tight text-fg tabular-nums">
            {fmtNumber(health?.usage?.requests)}
          </div>
          <div className="mt-2 flex items-center justify-between text-xs text-fg-muted">
            <span>{fmtNumber(health?.usage?.total_tokens)} tokens</span>
            <span className="font-mono font-medium text-fg">
              {fmtCost(health?.usage?.cost_usd)}
            </span>
          </div>
        </Card>

        <Card className="p-5">
          <div className="flex items-center justify-between text-xs font-medium text-fg-muted">
            <span>Cache &amp; Compression</span>
            <Sparkles className="h-4 w-4 text-success" />
          </div>
          <div className="mt-2.5 flex items-baseline gap-2">
            <span className="text-3xl font-semibold tracking-tight text-fg tabular-nums">
              {hitRate.toFixed(1)}%
            </span>
            <Badge variant={health?.cache?.config?.enabled ? "success" : "default"}>
              {health?.cache?.config?.enabled ? "CACHE ON" : "CACHE OFF"}
            </Badge>
          </div>
          <div className="mt-3 h-1.5 w-full overflow-hidden rounded-full bg-surface-muted">
            <div
              className="h-full rounded-full bg-accent transition-all duration-300"
              style={{ width: `${Math.min(100, Math.max(4, hitRate))}%` }}
            />
          </div>
          <div className="mt-2 flex items-center justify-between text-xs text-fg-muted">
            <span>{fmtNumber(compSaved)} tok saved</span>
            <span>{cacheEntries} cached</span>
          </div>
        </Card>

        <Card className="p-5">
          <div className="flex items-center justify-between text-xs font-medium text-fg-muted">
            <span>Operator Rank</span>
            <Trophy className="h-4 w-4 text-warning" />
          </div>
          <div className="mt-2.5 flex items-baseline gap-2">
            <span className="text-3xl font-semibold tracking-tight text-fg tabular-nums">
              Lv.{gamification?.level ?? 1}
            </span>
            <span className="text-xs font-medium text-accent-ink">
              {gamification?.tier || "Novice"}
            </span>
          </div>
          <div className="mt-3 h-1.5 w-full overflow-hidden rounded-full bg-surface-muted">
            <div
              className="h-full rounded-full bg-success transition-all duration-300"
              style={{ width: `${xpPct}%` }}
            />
          </div>
          <div className="mt-2 flex items-center justify-between text-xs text-fg-muted">
            <span>{fmtNumber(gamification?.total_xp)} XP</span>
            <span>{gamification?.current_streak ?? 0}d streak</span>
          </div>
        </Card>
      </div>

      {/* Quick OAuth Connect Strip */}
      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <KeyRound className="h-4 w-4 text-accent-ink" />
            <CardTitle>Quick OAuth Login</CardTitle>
            <Badge variant="accent">1-Click Redirect</Badge>
          </div>
          <Button variant="ghost" size="sm" onClick={() => onNavigate("oauth")}>
            View all 21 OAuth providers
            <ArrowUpRight className="h-3.5 w-3.5" />
          </Button>
        </CardHeader>
        <CardContent>
          <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-3 lg:grid-cols-6">
            {QUICK_OAUTH.map((item) => {
              const connectedCount = credentials.filter((c) => c.provider === item.id).length;
              return (
                <button
                  key={item.id}
                  type="button"
                  onClick={() => onLaunchOAuth(item.id, item.name)}
                  className="group flex flex-col items-start gap-2.5 rounded-xl border border-border bg-surface-subtle/50 p-3 text-left transition-all hover:border-accent/60 hover:bg-surface-subtle cursor-pointer"
                >
                  <div className="flex w-full items-center justify-between">
                    <div className="flex h-9 w-9 items-center justify-center rounded-lg border border-border bg-surface">
                      <ProviderIcon providerId={item.id} size={20} isDark={isDark} />
                    </div>
                    {connectedCount > 0 ? (
                      <Badge variant="success">{connectedCount}</Badge>
                    ) : (
                      <ArrowUpRight className="h-3.5 w-3.5 text-fg-muted transition-transform group-hover:translate-x-0.5 group-hover:-translate-y-0.5 group-hover:text-accent-ink" />
                    )}
                  </div>
                  <div>
                    <div className="text-xs font-semibold text-fg">{item.name}</div>
                    <div className="font-mono text-[10px] text-fg-muted">{item.id}</div>
                  </div>
                </button>
              );
            })}
          </div>
        </CardContent>
      </Card>

      {/* Main Split: Quick Connect SDK + Connected Accounts / Health */}
      <div className="grid grid-cols-1 gap-5 lg:grid-cols-5">
        {/* Quick Connect SDK (3 cols) */}
        <Card className="lg:col-span-3">
          <CardHeader className="flex-wrap">
            <div className="flex items-center gap-2">
              <Terminal className="h-4 w-4 text-accent-ink" />
              <CardTitle>Connect Your Apps &amp; Agents</CardTitle>
            </div>
            <SegmentedControl
              value={sdkTab}
              onChange={setSdkTab}
              options={[
                { value: "openai", label: "OpenAI SDK" },
                { value: "claude", label: "Claude Code" },
                { value: "curl", label: "cURL" },
                { value: "mcp", label: "MCP" },
              ]}
            />
          </CardHeader>
          <CardContent className="space-y-4">
            {/* Inline 1-Click API Key Bar */}
            <div className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-border bg-surface-subtle/50 px-3 py-2.5">
              <div className="flex items-center gap-2 min-w-0">
                <KeyRound className="h-3.5 w-3.5 shrink-0 text-accent-ink" />
                <Badge variant={hasRealKey ? "accent" : "success"}>
                  {hasRealKey ? "API Key" : "Open Local"}
                </Badge>
                <code className="truncate font-mono text-xs text-fg">{apiKey}</code>
              </div>
              <div className="flex items-center gap-1.5">
                <Button
                  variant="default"
                  size="sm"
                  onClick={async () => {
                    const ok = await copyToClipboard(apiKey);
                    onToast(ok ? "API key copied" : "Copy failed", ok ? "info" : "error");
                  }}
                >
                  <Copy className="h-3.5 w-3.5" /> Copy Key
                </Button>
                <Button
                  variant="primary"
                  size="sm"
                  disabled={genKeyBusy}
                  onClick={() => void quickCreateKey()}
                >
                  <Plus className="h-3.5 w-3.5" />
                  {genKeyBusy ? "Creating..." : "New Key"}
                </Button>
                {onOpenApiKeys && (
                  <Button variant="ghost" size="sm" onClick={onOpenApiKeys}>
                    Manage
                  </Button>
                )}
              </div>
            </div>

            <div className="relative overflow-hidden rounded-xl border border-border bg-[#0d100e] text-[#eef2ee]">
              <button
                type="button"
                onClick={copySnippet}
                className="absolute top-2.5 right-2.5 inline-flex items-center gap-1.5 rounded-md border border-white/15 bg-white/10 px-2.5 py-1 text-[11px] font-medium text-white hover:bg-white/20 cursor-pointer"
              >
                {copied ? <Check className="h-3.5 w-3.5 text-emerald-400" /> : <Copy className="h-3.5 w-3.5" />}
                {copied ? "Copied" : "Copy"}
              </button>
              <pre className="overflow-x-auto p-4 pr-24 font-mono text-xs leading-relaxed">
                <code>{snippets[sdkTab]}</code>
              </pre>
            </div>

            <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
              {[
                { label: "OpenAI Base URL", url: `${origin}/v1` },
                { label: "Anthropic Base URL", url: origin },
                { label: "MCP JSON-RPC", url: `${origin}/api/mcp` },
              ].map((ep) => (
                <button
                  key={ep.label}
                  type="button"
                  onClick={async () => {
                    const ok = await copyToClipboard(ep.url);
                    onToast(ok ? `Copied ${ep.label}` : "Copy failed", ok ? "info" : "error");
                  }}
                  className="flex flex-col rounded-lg border border-border bg-surface-subtle/60 px-3 py-2 text-left transition-colors hover:border-border-strong cursor-pointer"
                >
                  <span className="text-[10px] font-medium uppercase tracking-wider text-fg-muted">
                    {ep.label}
                  </span>
                  <span className="mt-0.5 truncate font-mono text-xs text-fg">{ep.url}</span>
                </button>
              ))}
            </div>
          </CardContent>
        </Card>

        {/* Right Column: Connected Accounts & Circuit Breakers (2 cols) */}
        <div className="space-y-5 lg:col-span-2">
          <Card>
            <CardHeader>
              <CardTitle>Connected Accounts</CardTitle>
              <Badge variant={credentials.length > 0 ? "success" : "default"}>
                {credentials.length} active
              </Badge>
            </CardHeader>
            <CardContent>
              {credentials.length === 0 ? (
                <EmptyState
                  title="No accounts connected yet"
                  description="Login with OAuth above or add an API key to start routing."
                  action={
                    <Button variant="primary" size="sm" onClick={() => onNavigate("providers")}>
                      Browse Providers
                    </Button>
                  }
                />
              ) : (
                <div className="space-y-2">
                  {credentials.slice(0, 5).map((c) => (
                    <div
                      key={`${c.provider}:${c.connection_id}`}
                      className="flex items-center justify-between gap-2.5 rounded-lg border border-border bg-surface-subtle/50 px-3 py-2"
                    >
                      <div className="flex items-center gap-2.5 min-w-0">
                        <ProviderIcon providerId={c.provider} size={20} isDark={isDark} />
                        <div className="min-w-0">
                          <div className="truncate text-xs font-semibold text-fg">
                            {c.label || c.provider}
                          </div>
                          <div className="truncate font-mono text-[10px] text-fg-muted">
                            {c.provider} · {c.auth_type}
                          </div>
                        </div>
                      </div>
                      <Badge variant={c.health_status === "healthy" ? "success" : "danger"}>
                        {c.health_status || "healthy"}
                      </Badge>
                    </div>
                  ))}
                  {credentials.length > 5 && (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="w-full"
                      onClick={() => onNavigate("providers")}
                    >
                      View all {credentials.length} accounts
                    </Button>
                  )}
                </div>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <div className="flex items-center gap-2">
                <ShieldCheck className="h-4 w-4 text-success" />
                <CardTitle>Circuit Breakers &amp; Resilience</CardTitle>
              </div>
              <Badge variant={lockouts.length ? "warning" : "success"}>
                {lockouts.length} lockouts
              </Badge>
            </CardHeader>
            <CardContent>
              {breakers.length === 0 && lockouts.length === 0 ? (
                <div className="rounded-xl border border-success/25 bg-success-bg/50 px-4 py-3 text-xs text-success">
                  All provider circuits are CLOSED and healthy. Zero model lockouts active.
                </div>
              ) : (
                <div className="space-y-1.5">
                  {breakers.map((b) => (
                    <div
                      key={b.provider}
                      className="flex items-center justify-between rounded-lg border border-border bg-surface-subtle/40 px-3 py-2 text-xs"
                    >
                      <span className="font-mono font-medium text-fg">{b.provider}</span>
                      <div className="flex items-center gap-2">
                        <span className="font-mono text-fg-muted">streak {b.failure_streak}</span>
                        <Badge
                          variant={
                            b.state === "CLOSED"
                              ? "success"
                              : b.state === "OPEN"
                                ? "danger"
                                : "warning"
                          }
                        >
                          {b.state}
                        </Badge>
                      </div>
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
