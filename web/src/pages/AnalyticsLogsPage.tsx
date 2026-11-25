import { useCallback, useEffect, useState } from "react";
import {
  Activity,
  Download,
  Trash2,
  DollarSign,
  Plus,
  RefreshCw,
  Search,
  AlertTriangle,
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
  Select,
  StatCard,
  PageHeader,
  EmptyState,
} from "@/components/ui/primitives";
import { ProviderIcon } from "@/components/ProviderIcon";
import { api, apiDelete, apiPost } from "@/lib/api";
import { fmtNum } from "@/lib/utils";

export function AnalyticsLogsPage({
  isDark,
  onToast,
}: {
  isDark: boolean;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [days, setDays] = useState(14);
  const [analytics, setAnalytics] = useState<any>(null);
  const [logs, setLogs] = useState<any[]>([]);
  const [budgets, setBudgets] = useState<any[]>([]);
  const [loading, setLoading] = useState(false);

  // Filters
  const [filterProvider, setFilterProvider] = useState("");
  const [filterStatus, setFilterStatus] = useState("");
  const [filterQuery, setFilterQuery] = useState("");

  // New budget form
  const [scopeId, setScopeId] = useState("global");
  const [dailyLimit, setDailyLimit] = useState("5");
  const [weeklyLimit, setWeeklyLimit] = useState("25");
  const [monthlyLimit, setMonthlyLimit] = useState("100");
  const [warnPct, setWarnPct] = useState("80");

  const loadAll = useCallback(async () => {
    setLoading(true);
    const params = new URLSearchParams({ limit: "150" });
    if (filterProvider.trim()) params.set("provider", filterProvider.trim());
    if (filterStatus) params.set("status", filterStatus);
    if (filterQuery.trim()) params.set("q", filterQuery.trim());

    const [aRes, lRes, bRes] = await Promise.all([
      api<any>(`/api/analytics?days=${days}`),
      api<any>(`/api/logs?${params.toString()}`),
      api<any>("/api/budgets"),
    ]);
    setLoading(false);
    if (aRes.ok) setAnalytics(aRes.data);
    if (lRes.ok) setLogs(lRes.data?.logs || []);
    if (bRes.ok) setBudgets(bRes.data?.budgets || []);
  }, [days, filterProvider, filterStatus, filterQuery]);

  useEffect(() => {
    void loadAll();
  }, [loadAll]);

  const saveBudget = async () => {
    const r = await apiPost("/api/budgets", {
      api_key_id: scopeId.trim() || "global",
      daily_limit_usd: Number.parseFloat(dailyLimit) || 0,
      weekly_limit_usd: Number.parseFloat(weeklyLimit) || 0,
      monthly_limit_usd: Number.parseFloat(monthlyLimit) || 0,
      warning_threshold: Math.min(1, Math.max(0.1, (Number.parseFloat(warnPct) || 80) / 100)),
    });
    if (r.ok) {
      onToast(`Budget cap saved for ${scopeId.trim() || "global"}`, "success");
      void loadAll();
    } else {
      onToast(r.error || "Failed to save budget cap", "error");
    }
  };

  const removeBudget = async (id: string) => {
    const r = await apiDelete(`/api/budgets/${encodeURIComponent(id)}`);
    if (r.ok) {
      onToast(`Removed budget cap for ${id}`, "info");
      void loadAll();
    } else {
      onToast(r.error || "Delete failed", "error");
    }
  };

  const clearLogs = async () => {
    const r = await apiDelete("/api/logs");
    if (r.ok) {
      onToast(`Cleared ${r.data?.deleted || 0} log entries`, "info");
      void loadAll();
    } else {
      onToast(r.error || "Failed to clear logs", "error");
    }
  };

  const daily: any[] = analytics?.daily || [];
  const maxReqs = Math.max(1, ...daily.map((d) => Number(d.requests || 0)));
  const byModel: any[] = analytics?.by_model || [];
  const win = analytics?.window || {};
  const spend = analytics?.spend || {};

  return (
    <div className="space-y-6">
      <PageHeader
        title="Analytics, Call Logs & Budget Caps"
        description="Daily request/cost charts, p50/p95 latency, hard spend enforcement & downloadable request logs."
        actions={
          <div className="flex items-center gap-2">
            <Select value={String(days)} onChange={(e) => setDays(Number(e.target.value))}>
              <option value="7">Last 7 days</option>
              <option value="14">Last 14 days</option>
              <option value="30">Last 30 days</option>
            </Select>
            <Button variant="default" size="sm" onClick={() => void loadAll()}>
              <RefreshCw className={`h-3.5 w-3.5 ${loading ? "animate-spin" : ""}`} />
              Refresh
            </Button>
          </div>
        }
      />

      {/* Top KPI Strip */}
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <StatCard
          label={`Requests (${days}d)`}
          value={fmtNum(win.requests || 0)}
          sub={`${fmtNum(win.errors || 0)} errors`}
          icon={<Activity className="h-4 w-4 text-accent-ink" />}
        />
        <StatCard
          label={`Tokens (${days}d)`}
          value={fmtNum(win.total_tokens || 0)}
          sub={`p50 ${win.p50_latency_ms || 0}ms · p95 ${win.p95_latency_ms || 0}ms`}
        />
        <StatCard
          label="24h / 7d Spend"
          value={`$${Number(spend.daily_usd || 0).toFixed(4)}`}
          sub={`7d: $${Number(spend.weekly_usd || 0).toFixed(4)}`}
          icon={<DollarSign className="h-4 w-4 text-success" />}
        />
        <StatCard
          label="30d Spend"
          value={`$${Number(spend.monthly_usd || 0).toFixed(4)}`}
          sub={`${budgets.length} active budget cap(s)`}
        />
      </div>

      {/* Daily Requests & Cost Chart + Top Models */}
      <div className="grid grid-cols-1 gap-5 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader>
            <div>
              <CardTitle>Daily Traffic &amp; Cost ({days}d)</CardTitle>
              <CardDescription>Requests per day (accent bar) with error overlay (red)</CardDescription>
            </div>
            <Badge variant="accent">{daily.length} active day(s)</Badge>
          </CardHeader>
          <CardContent>
            {daily.length === 0 ? (
              <EmptyState
                title="No usage data in selected window"
                description="Send requests via /v1/chat/completions or Playground to populate daily charts."
              />
            ) : (
              <div className="space-y-3">
                <div className="w-full overflow-x-auto">
                  <svg
                    viewBox={`0 0 ${Math.max(420, daily.length * 42)} 180`}
                    className="h-48 w-full min-w-[360px]"
                  >
                    {[0.25, 0.5, 0.75, 1].map((frac) => (
                      <line
                        key={frac}
                        x1={0}
                        y1={145 - frac * 125}
                        x2={Math.max(420, daily.length * 42)}
                        y2={145 - frac * 125}
                        stroke="currentColor"
                        strokeOpacity={0.09}
                        strokeDasharray="3 3"
                      />
                    ))}
                    {daily.map((d, idx) => {
                      const reqs = Number(d.requests || 0);
                      const errs = Number(d.errors || 0);
                      const h = Math.max(4, Math.round((reqs / maxReqs) * 125));
                      const eh = errs > 0 ? Math.max(3, Math.round((errs / maxReqs) * 125)) : 0;
                      const x = idx * 42 + 10;
                      return (
                        <g key={d.date}>
                          <title>
                            {`${d.date}: ${reqs} reqs, ${errs} errs, ${fmtNum(d.total_tokens)} tok, $${Number(d.cost_usd || 0).toFixed(4)}`}
                          </title>
                          <rect
                            x={x}
                            y={145 - h}
                            width={24}
                            height={h}
                            rx={5}
                            className="fill-accent opacity-90 hover:opacity-100 transition-opacity"
                          />
                          {eh > 0 && (
                            <rect
                              x={x}
                              y={145 - eh}
                              width={24}
                              height={eh}
                              rx={4}
                              className="fill-danger"
                            />
                          )}
                          <text
                            x={x + 12}
                            y={164}
                            textAnchor="middle"
                            className="fill-fg-muted text-[9px] font-mono"
                          >
                            {String(d.date).slice(5)}
                          </text>
                        </g>
                      );
                    })}
                  </svg>
                </div>
              </div>
            )}
          </CardContent>
        </Card>

        {/* Top Models Breakdown */}
        <Card>
          <CardHeader>
            <CardTitle>Top Models</CardTitle>
            <Badge variant="default">{byModel.length}</Badge>
          </CardHeader>
          <CardContent>
            {byModel.length === 0 ? (
              <EmptyState title="No model stats yet" description="Usage breakdown appears here." />
            ) : (
              <div className="space-y-2.5 max-h-56 overflow-y-auto pr-1">
                {byModel.slice(0, 10).map((m, i) => (
                  <div
                    key={i}
                    className="flex items-center justify-between gap-2 rounded-lg border border-border bg-surface-subtle/40 px-2.5 py-2"
                  >
                    <div className="flex items-center gap-2 min-w-0">
                      <ProviderIcon providerId={m.provider} size={18} isDark={isDark} />
                      <div className="min-w-0">
                        <div className="truncate font-mono text-[11px] font-medium text-fg">
                          {m.model}
                        </div>
                        <div className="text-[10px] text-fg-muted">
                          {m.provider} · {m.avg_latency_ms}ms
                        </div>
                      </div>
                    </div>
                    <div className="text-right shrink-0">
                      <div className="font-mono text-xs font-semibold text-fg">
                        {fmtNum(m.requests)}
                      </div>
                      <div className="font-mono text-[10px] text-fg-muted">
                        ${Number(m.cost_usd || 0).toFixed(4)}
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      </div>

      {/* Hard Cost / Budget Caps */}
      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <AlertTriangle className="h-4 w-4 text-warning" />
            <div>
              <CardTitle>Hard Cost &amp; Budget Caps</CardTitle>
              <CardDescription>
                Enforce HTTP 402 hard limits per API key hash or <code className="font-mono">global</code> (fires Webhook/Telegram alerts on warning &amp; breach)
              </CardDescription>
            </div>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid grid-cols-2 gap-2 sm:grid-cols-6">
            <Input
              value={scopeId}
              onChange={(e) => setScopeId(e.target.value)}
              placeholder="global or key_hash"
              className="col-span-2 font-mono text-xs"
            />
            <Input
              type="number"
              step="0.5"
              value={dailyLimit}
              onChange={(e) => setDailyLimit(e.target.value)}
              placeholder="Daily $"
            />
            <Input
              type="number"
              step="1"
              value={weeklyLimit}
              onChange={(e) => setWeeklyLimit(e.target.value)}
              placeholder="Weekly $"
            />
            <Input
              type="number"
              step="5"
              value={monthlyLimit}
              onChange={(e) => setMonthlyLimit(e.target.value)}
              placeholder="Monthly $"
            />
            <div className="flex gap-1.5">
              <Input
                type="number"
                value={warnPct}
                onChange={(e) => setWarnPct(e.target.value)}
                placeholder="Warn %"
                title="Warning threshold %"
              />
              <Button variant="primary" size="sm" onClick={saveBudget}>
                <Plus className="h-3.5 w-3.5" />
                Save
              </Button>
            </div>
          </div>

          {budgets.length > 0 && (
            <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
              {budgets.map((b) => {
                const dailyPct =
                  b.daily_limit_usd > 0
                    ? Math.min(100, (b.daily_spent_usd / b.daily_limit_usd) * 100)
                    : 0;
                const over = b.daily_limit_usd > 0 && b.daily_spent_usd >= b.daily_limit_usd;
                return (
                  <div
                    key={b.api_key_id}
                    className="rounded-xl border border-border bg-surface-subtle/35 p-3.5 space-y-2"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <div className="flex items-center gap-2">
                        <Badge variant={over ? "danger" : "accent"} className="font-mono">
                          {b.api_key_id}
                        </Badge>
                        <span className="text-[11px] text-fg-muted">
                          Warn at {Math.round((b.warning_threshold || 0.8) * 100)}%
                        </span>
                      </div>
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => void removeBudget(b.api_key_id)}
                        title="Delete cap"
                      >
                        <Trash2 className="h-3.5 w-3.5 text-danger" />
                      </Button>
                    </div>
                    <div className="h-2 w-full overflow-hidden rounded-full bg-surface">
                      <div
                        className={`h-full rounded-full transition-all ${over ? "bg-danger" : "bg-accent"}`}
                        style={{ width: `${dailyPct}%` }}
                      />
                    </div>
                    <div className="flex flex-wrap justify-between gap-2 font-mono text-[11px] text-fg-muted">
                      <span>
                        Day: ${Number(b.daily_spent_usd || 0).toFixed(4)} / ${b.daily_limit_usd}
                      </span>
                      <span>
                        Week: ${Number(b.weekly_spent_usd || 0).toFixed(4)} / ${b.weekly_limit_usd}
                      </span>
                      <span>
                        Month: ${Number(b.monthly_spent_usd || 0).toFixed(4)} / ${b.monthly_limit_usd}
                      </span>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>

      {/* Call Logs Inspector */}
      <Card>
        <CardHeader>
          <div>
            <CardTitle>Call Logs Inspector</CardTitle>
            <CardDescription>Bounded ring buffer (latest 2,000 calls) optimized for low-eMMC STB storage</CardDescription>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <a
              href="/api/logs/export?format=csv"
              className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-border-strong bg-surface px-2.5 text-xs font-medium text-fg hover:bg-surface-subtle"
            >
              <Download className="h-3.5 w-3.5" /> CSV
            </a>
            <a
              href="/api/logs/export?format=jsonl"
              className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-border-strong bg-surface px-2.5 text-xs font-medium text-fg hover:bg-surface-subtle"
            >
              <Download className="h-3.5 w-3.5" /> JSONL
            </a>
            <Button variant="danger" size="sm" onClick={clearLogs}>
              <Trash2 className="h-3.5 w-3.5" /> Clear
            </Button>
          </div>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex flex-wrap gap-2">
            <div className="relative flex-1 min-w-[180px]">
              <Search className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-fg-muted" />
              <Input
                value={filterQuery}
                onChange={(e) => setFilterQuery(e.target.value)}
                placeholder="Search model, connection, or error..."
                className="pl-8"
              />
            </div>
            <Input
              value={filterProvider}
              onChange={(e) => setFilterProvider(e.target.value)}
              placeholder="Provider (e.g. antigravity)"
              className="w-44"
            />
            <Select value={filterStatus} onChange={(e) => setFilterStatus(e.target.value)}>
              <option value="">All statuses</option>
              <option value="2xx">2xx Success</option>
              <option value="4xx">4xx Client / Quota</option>
              <option value="5xx">5xx Upstream Error</option>
            </Select>
          </div>

          {logs.length === 0 ? (
            <EmptyState
              title="No call logs match your filter"
              description="Inference calls through /v1/* are automatically recorded here (unless x-omniroute-no-log is set)."
            />
          ) : (
            <div className="overflow-x-auto rounded-lg border border-border">
              <table className="w-full text-left text-xs">
                <thead className="bg-surface-subtle text-fg-muted">
                  <tr>
                    <th className="px-3 py-2 font-medium">Time</th>
                    <th className="px-3 py-2 font-medium">Status</th>
                    <th className="px-3 py-2 font-medium">Provider / Account</th>
                    <th className="px-3 py-2 font-medium">Model</th>
                    <th className="px-3 py-2 font-medium">Tokens</th>
                    <th className="px-3 py-2 font-medium">Latency</th>
                    <th className="px-3 py-2 font-medium">Error</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-border">
                  {logs.map((l) => (
                    <tr key={l.id} className="hover:bg-surface-subtle/40">
                      <td className="px-3 py-2 font-mono text-[11px] text-fg-muted whitespace-nowrap">
                        {new Date(l.timestamp).toLocaleTimeString()}
                      </td>
                      <td className="px-3 py-2">
                        <Badge
                          variant={
                            l.status >= 200 && l.status < 300
                              ? "success"
                              : l.status === 429 || l.status === 402
                                ? "warning"
                                : "danger"
                          }
                          className="font-mono"
                        >
                          {l.status}
                        </Badge>
                      </td>
                      <td className="px-3 py-2">
                        <div className="flex items-center gap-1.5">
                          <ProviderIcon providerId={l.provider} size={16} isDark={isDark} />
                          <span className="font-medium text-fg">{l.provider}</span>
                          {l.connection_id && (
                            <span className="font-mono text-[10px] text-fg-muted">
                              ({l.connection_id})
                            </span>
                          )}
                        </div>
                      </td>
                      <td className="px-3 py-2 font-mono text-[11px] text-fg">{l.model}</td>
                      <td className="px-3 py-2 font-mono text-[11px] text-fg-muted">
                        {l.tokens_in} / {l.tokens_out}
                      </td>
                      <td className="px-3 py-2 font-mono text-[11px]">{l.duration}ms</td>
                      <td className="px-3 py-2 max-w-[240px] truncate font-mono text-[11px] text-danger">
                        {l.error_summary || "—"}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
