import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Activity,
  Download,
  Trash2,
  DollarSign,
  Plus,
  RefreshCw,
  Search,
  AlertTriangle,
  ScrollText,
  BarChart3,
  ShieldAlert,
  Copy,
  Check,
  Clock,
  ChevronRight,
  ChevronLeft,
  Radio,
  Filter,
  X,
  CheckCircle2,
  XCircle,
  KeyRound,
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
  Dialog,
  StatCard,
  PageHeader,
  EmptyState,
} from "@/components/ui/primitives";
import { ProviderIcon } from "@/components/ProviderIcon";
import { api, apiDelete, apiPost, getStoredApiKey } from "@/lib/api";
import { cn, copyToClipboard, fmtNum } from "@/lib/utils";

type ActiveTab = "logs" | "analytics" | "budgets";
type StatusQuickFilter = "" | "2xx" | "429" | "4xx" | "5xx" | "errors";

interface CallLogItem {
  id: string;
  timestamp: string;
  method?: string;
  path?: string;
  status: number;
  model: string;
  provider: string;
  connection_id?: string;
  duration: number;
  tokens_in: number;
  tokens_out: number;
  error_summary?: string | null;
}

function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "0ms";
  if (ms >= 10_000) return `${(ms / 1000).toFixed(1)}s`;
  if (ms >= 1_000) return `${(ms / 1000).toFixed(2)}s`;
  return `${Math.round(ms)}ms`;
}

function formatRelativeTime(ts: string): string {
  const t = new Date(ts).getTime();
  if (!Number.isFinite(t)) return "";
  const diffSec = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (diffSec < 5) return "just now";
  if (diffSec < 60) return `${diffSec}s ago`;
  const m = Math.floor(diffSec / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

function formatClock(ts: string): string {
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return ts;
  return d.toLocaleTimeString([], { hour12: false, hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function formatFullDate(ts: string): string {
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return ts;
  return d.toLocaleString([], {
    year: "numeric",
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

function deepUnwrapJson(input: unknown, depth = 0): unknown {
  if (depth > 4) return input;
  if (typeof input === "string") {
    const trimmed = input.trim();
    if ((trimmed.startsWith("{") && trimmed.endsWith("}")) || (trimmed.startsWith("[") && trimmed.endsWith("]"))) {
      try {
        return deepUnwrapJson(JSON.parse(trimmed), depth + 1);
      } catch {
        // Also handle trailing notes like `{"object":"error",...} (reset after 31s)`
        const lastBrace = trimmed.lastIndexOf("}");
        if (trimmed.startsWith("{") && lastBrace > 0 && lastBrace < trimmed.length - 1) {
          try {
            const parsed = JSON.parse(trimmed.slice(0, lastBrace + 1));
            const suffix = trimmed.slice(lastBrace + 1).trim();
            return {
              ...((typeof parsed === "object" && parsed !== null) ? (deepUnwrapJson(parsed, depth + 1) as object) : { value: parsed }),
              _note: suffix,
            };
          } catch {
            return input;
          }
        }
        return input;
      }
    }
    return input;
  }
  if (Array.isArray(input)) {
    return input.map((v) => deepUnwrapJson(v, depth + 1));
  }
  if (input && typeof input === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(input as Record<string, unknown>)) {
      out[k] = deepUnwrapJson(v, depth + 1);
    }
    return out;
  }
  return input;
}

function extractReadableError(raw?: string | null): string {
  if (!raw) return "";
  const unwrapped = deepUnwrapJson(raw);
  if (typeof unwrapped === "string") return unwrapped.replace(/\s+/g, " ").trim();
  if (unwrapped && typeof unwrapped === "object") {
    const obj = unwrapped as any;
    const err = obj.error ?? obj;
    if (err && typeof err === "object") {
      // Pydantic detail array: { detail: [{ type, loc, msg }] }
      const detail = err.detail ?? err.message?.detail;
      if (Array.isArray(detail) && detail.length > 0) {
        const d0 = detail[0];
        const loc = Array.isArray(d0?.loc) ? d0.loc.join(".") : "";
        return `${d0?.type || "validation"}: ${d0?.msg || ""}${loc ? ` (${loc})` : ""}`.trim();
      }
      if (typeof err.message === "object" && err.message !== null) {
        const innerMsg = err.message.message || err.message.error || err.message.type;
        const note = err.message._note ? ` ${err.message._note}` : "";
        if (innerMsg) return `${innerMsg}${note}`.trim();
      }
      if (typeof err.message === "string" && err.message.trim()) {
        const status = err.status ? `[${err.status}] ` : "";
        const note = err._note ? ` ${err._note}` : "";
        return `${status}${err.message.trim()}${note}`.trim();
      }
    }
    return JSON.stringify(unwrapped);
  }
  return String(raw);
}

function formatPrettyError(raw?: string | null): string {
  if (!raw) return "";
  const unwrapped = deepUnwrapJson(raw);
  if (typeof unwrapped === "string") return unwrapped;
  try {
    return JSON.stringify(unwrapped, null, 2);
  } catch {
    return raw;
  }
}

function statusMeta(status: number): {
  variant: "success" | "warning" | "danger";
  bar: string;
  label: string;
} {
  if (status >= 200 && status < 300) {
    return { variant: "success", bar: "bg-success", label: "OK" };
  }
  if (status === 429) {
    return { variant: "warning", bar: "bg-warning", label: "Quota" };
  }
  if (status === 402) {
    return { variant: "warning", bar: "bg-warning", label: "Budget" };
  }
  if (status >= 400 && status < 500) {
    return { variant: "danger", bar: "bg-danger", label: "Client" };
  }
  return { variant: "danger", bar: "bg-danger", label: "Upstream" };
}

export function AnalyticsLogsPage({
  isDark,
  onToast,
}: {
  isDark: boolean;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [activeTab, setActiveTab] = useState<ActiveTab>(() => {
    try {
      const saved = localStorage.getItem("omniroute_logs_subtab");
      if (saved === "logs" || saved === "analytics" || saved === "budgets") return saved;
    } catch {}
    return "logs";
  });

  useEffect(() => {
    try {
      localStorage.setItem("omniroute_logs_subtab", activeTab);
    } catch {}
  }, [activeTab]);
  const [days, setDays] = useState(14);
  const [analytics, setAnalytics] = useState<any>(null);
  const [logs, setLogs] = useState<CallLogItem[]>([]);
  const [budgets, setBudgets] = useState<any[]>([]);
  const [loading, setLoading] = useState(false);
  const [authRequired, setAuthRequired] = useState(false);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);

  // Live auto-refresh & pagination
  const [liveMode, setLiveMode] = useState(true);
  const [rowLimit, setRowLimit] = useState(150);
  const [confirmClear, setConfirmClear] = useState(false);

  // Filters
  const [filterProvider, setFilterProvider] = useState("");
  const [filterStatus, setFilterStatus] = useState<StatusQuickFilter>("");
  const [filterQuery, setFilterQuery] = useState("");

  // Inspector detail modal
  const [selectedLogId, setSelectedLogId] = useState<string | null>(null);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  // New budget form
  const [scopeId, setScopeId] = useState("global");
  const [dailyLimit, setDailyLimit] = useState("5");
  const [weeklyLimit, setWeeklyLimit] = useState("25");
  const [monthlyLimit, setMonthlyLimit] = useState("100");
  const [warnPct, setWarnPct] = useState("80");

  const loadAll = useCallback(
    async (silent = false) => {
      if (!silent) setLoading(true);
      const params = new URLSearchParams({ limit: String(rowLimit) });
      if (filterProvider.trim()) params.set("provider", filterProvider.trim());
      if (filterStatus && filterStatus !== "429" && filterStatus !== "errors") {
        params.set("status", filterStatus);
      }
      if (filterQuery.trim()) params.set("q", filterQuery.trim());

      const [aRes, lRes, bRes] = await Promise.all([
        api<any>(`/api/analytics?days=${days}`),
        api<any>(`/api/logs?${params.toString()}`),
        api<any>("/api/budgets"),
      ]);
      if (!silent) setLoading(false);

      if (lRes.status === 401 || aRes.status === 401) {
        setAuthRequired(true);
        return;
      }
      setAuthRequired(false);
      if (aRes.ok) setAnalytics(aRes.data);
      if (lRes.ok) setLogs(Array.isArray(lRes.data?.logs) ? lRes.data.logs : []);
      if (bRes.ok) setBudgets(Array.isArray(bRes.data?.budgets) ? bRes.data.budgets : []);
      setLastUpdated(new Date());
    },
    [days, rowLimit, filterProvider, filterStatus, filterQuery]
  );

  useEffect(() => {
    void loadAll();
  }, [loadAll]);

  // Live 5s polling when on Logs tab and no detail modal open
  useEffect(() => {
    if (!liveMode || activeTab !== "logs" || selectedLogId) return;
    const id = window.setInterval(() => {
      void loadAll(true);
    }, 5000);
    return () => window.clearInterval(id);
  }, [liveMode, activeTab, selectedLogId, loadAll]);

  // Client-side refinement for 429 & errors-only quick pills
  const displayedLogs = useMemo(() => {
    if (filterStatus === "429") {
      return logs.filter((l) => l.status === 429);
    }
    if (filterStatus === "errors") {
      return logs.filter((l) => l.status < 200 || l.status >= 400);
    }
    return logs;
  }, [logs, filterStatus]);

  // Known providers for filter dropdown
  const providerOptions = useMemo(() => {
    const set = new Set<string>();
    logs.forEach((l) => {
      if (l.provider) set.add(l.provider);
    });
    (analytics?.by_model || []).forEach((m: any) => {
      if (m.provider) set.add(m.provider);
    });
    return Array.from(set).sort();
  }, [logs, analytics]);

  // Live metrics calculated over currently loaded logs
  const logStats = useMemo(() => {
    const total = displayedLogs.length;
    let ok = 0;
    let rateLimited = 0;
    let clientErr = 0;
    let serverErr = 0;
    let totalIn = 0;
    let totalOut = 0;
    const durs: number[] = [];

    for (const l of displayedLogs) {
      if (l.status >= 200 && l.status < 300) ok++;
      else if (l.status === 429) rateLimited++;
      else if (l.status >= 400 && l.status < 500) clientErr++;
      else serverErr++;

      totalIn += Number(l.tokens_in || 0);
      totalOut += Number(l.tokens_out || 0);
      if (Number.isFinite(l.duration) && l.duration >= 0) durs.push(l.duration);
    }
    durs.sort((a, b) => a - b);
    const p50 = durs.length ? durs[Math.floor(durs.length * 0.5)] : 0;
    const p95 = durs.length ? durs[Math.min(durs.length - 1, Math.floor(durs.length * 0.95))] : 0;
    const avg = durs.length ? Math.round(durs.reduce((a, b) => a + b, 0) / durs.length) : 0;
    const errors = total - ok;
    const successRate = total > 0 ? (ok / total) * 100 : 100;
    return { total, ok, errors, rateLimited, clientErr, serverErr, totalIn, totalOut, avg, p50, p95, successRate };
  }, [displayedLogs]);

  // Counts across unfiltered fetched batch for quick pills
  const batchCounts = useMemo(() => {
    let ok = 0;
    let q429 = 0;
    let err = 0;
    for (const l of logs) {
      if (l.status >= 200 && l.status < 300) ok++;
      else {
        err++;
        if (l.status === 429) q429++;
      }
    }
    return { all: logs.length, ok, q429, err };
  }, [logs]);

  const selectedIndex = useMemo(
    () => (selectedLogId ? displayedLogs.findIndex((l) => l.id === selectedLogId) : -1),
    [displayedLogs, selectedLogId]
  );
  const selectedLog = selectedIndex >= 0 ? displayedLogs[selectedIndex] : null;

  const handleCopy = async (key: string, text: string, label: string) => {
    const ok = await copyToClipboard(text);
    if (ok) {
      setCopiedKey(key);
      window.setTimeout(() => setCopiedKey((prev) => (prev === key ? null : prev)), 1500);
      onToast(`Copied ${label}`, "info");
    } else {
      onToast("Copy failed", "error");
    }
  };

  const handleExport = async (format: "csv" | "jsonl") => {
    try {
      const headers: Record<string, string> = {};
      const key = getStoredApiKey();
      if (key) headers["Authorization"] = `Bearer ${key}`;
      const res = await fetch(`/api/logs/export?format=${format}`, {
        credentials: "same-origin",
        headers,
      });
      if (!res.ok) {
        onToast(`Export failed (HTTP ${res.status})`, "error");
        return;
      }
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `omniroute-call-logs-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.${format}`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
      onToast(`Exported call logs (${format.toUpperCase()})`, "success");
    } catch (e: any) {
      onToast(e?.message || "Export failed", "error");
    }
  };

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
    setConfirmClear(false);
    if (r.ok) {
      setSelectedLogId(null);
      onToast(`Cleared ${r.data?.deleted || 0} log entries`, "info");
      void loadAll();
    } else {
      onToast(r.error || "Failed to clear logs", "error");
    }
  };

  const resetFilters = () => {
    setFilterProvider("");
    setFilterStatus("");
    setFilterQuery("");
  };

  const hasActiveFilters = !!(filterProvider || filterStatus || filterQuery);

  const daily: any[] = analytics?.daily || [];
  const maxReqs = Math.max(1, ...daily.map((d) => Number(d.requests || 0)));
  const byModel: any[] = analytics?.by_model || [];
  const win = analytics?.window || {};
  const spend = analytics?.spend || {};

  return (
    <div className="space-y-5">
      <PageHeader
        title="Call Logs, Analytics & Budgets"
        description="Real-time request inspector with full JSON error diagnostics, latency percentiles & cost enforcement."
        actions={
          <div className="flex flex-wrap items-center gap-2">
            {activeTab === "logs" && (
              <button
                type="button"
                onClick={() => setLiveMode((v) => !v)}
                className={cn(
                  "inline-flex h-8 items-center gap-1.5 rounded-lg border px-2.5 text-xs font-medium transition-colors",
                  liveMode
                    ? "border-success/40 bg-success/10 text-success"
                    : "border-border-strong bg-surface text-fg-muted hover:text-fg"
                )}
                title="Toggle 5-second live log polling"
              >
                <span className="relative flex h-2 w-2">
                  {liveMode && (
                    <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-success opacity-75" />
                  )}
                  <span
                    className={cn(
                      "relative inline-flex h-2 w-2 rounded-full",
                      liveMode ? "bg-success" : "bg-fg-muted"
                    )}
                  />
                </span>
                <Radio className="h-3 w-3" />
                {liveMode ? "Live · 5s" : "Paused"}
              </button>
            )}
            {activeTab === "analytics" && (
              <Select value={String(days)} onChange={(e) => setDays(Number(e.target.value))}>
                <option value="7">Last 7 days</option>
                <option value="14">Last 14 days</option>
                <option value="30">Last 30 days</option>
              </Select>
            )}
            <Button variant="default" size="sm" onClick={() => void loadAll()}>
              <RefreshCw className={cn("h-3.5 w-3.5", loading && "animate-spin")} />
              Refresh
            </Button>
          </div>
        }
      />

      {authRequired && (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-warning/40 bg-warning/10 px-4 py-3 text-xs text-fg">
          <div className="flex items-center gap-2.5">
            <KeyRound className="h-4 w-4 shrink-0 text-warning" />
            <div>
              <span className="font-semibold">API key authentication is active.</span>{" "}
              <span className="text-fg-muted">
                Click <strong className="text-fg">Connect / Keys</strong> in the top header and enter your gateway API key to load live logs &amp; analytics.
              </span>
            </div>
          </div>
          <Button variant="default" size="sm" onClick={() => void loadAll()}>
            Retry
          </Button>
        </div>
      )}

      {/* Segmented Section Tabs */}
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border pb-3">
        <div className="inline-flex rounded-xl border border-border bg-surface-subtle/60 p-1">
          <button
            type="button"
            onClick={() => setActiveTab("logs")}
            className={cn(
              "inline-flex items-center gap-2 rounded-lg px-3.5 py-1.5 text-xs font-medium transition-all",
              activeTab === "logs"
                ? "bg-surface text-fg shadow-sm ring-1 ring-border"
                : "text-fg-muted hover:text-fg"
            )}
          >
            <ScrollText className="h-3.5 w-3.5" />
            Call Logs
            <Badge variant={batchCounts.err > 0 ? "warning" : "default"} className="font-mono text-[10px]">
              {batchCounts.all}
            </Badge>
          </button>
          <button
            type="button"
            onClick={() => setActiveTab("analytics")}
            className={cn(
              "inline-flex items-center gap-2 rounded-lg px-3.5 py-1.5 text-xs font-medium transition-all",
              activeTab === "analytics"
                ? "bg-surface text-fg shadow-sm ring-1 ring-border"
                : "text-fg-muted hover:text-fg"
            )}
          >
            <BarChart3 className="h-3.5 w-3.5" />
            Analytics &amp; Charts
            <Badge variant="default" className="font-mono text-[10px]">
              {fmtNum(win.requests || 0)}
            </Badge>
          </button>
          <button
            type="button"
            onClick={() => setActiveTab("budgets")}
            className={cn(
              "inline-flex items-center gap-2 rounded-lg px-3.5 py-1.5 text-xs font-medium transition-all",
              activeTab === "budgets"
                ? "bg-surface text-fg shadow-sm ring-1 ring-border"
                : "text-fg-muted hover:text-fg"
            )}
          >
            <ShieldAlert className="h-3.5 w-3.5" />
            Budget Caps
            {budgets.length > 0 && (
              <Badge variant="accent" className="font-mono text-[10px]">
                {budgets.length}
              </Badge>
            )}
          </button>
        </div>

        {lastUpdated && (
          <div className="flex items-center gap-1.5 font-mono text-[11px] text-fg-muted">
            <Clock className="h-3 w-3" />
            Updated {lastUpdated.toLocaleTimeString([], { hour12: false })}
          </div>
        )}
      </div>

      {/* ═══════════════════ TAB 1: CALL LOGS EXPLORER ═══════════════════ */}
      {activeTab === "logs" && (
        <div className="space-y-4">
          {/* Live Log Summary KPIs */}
          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <StatCard
              label="Visible Calls"
              value={fmtNum(logStats.total)}
              sub={`${fmtNum(logStats.ok)} ok · ${fmtNum(logStats.errors)} errors`}
              icon={<Activity className="h-4 w-4 text-accent-ink" />}
            />
            <StatCard
              label="Success Rate"
              value={`${logStats.successRate.toFixed(1)}%`}
              sub={
                logStats.rateLimited > 0
                  ? `${logStats.rateLimited} rate-limited (429)`
                  : logStats.serverErr > 0
                    ? `${logStats.serverErr} upstream 5xx`
                    : "All healthy"
              }
              icon={
                logStats.successRate >= 95 ? (
                  <CheckCircle2 className="h-4 w-4 text-success" />
                ) : (
                  <XCircle className="h-4 w-4 text-danger" />
                )
              }
            />
            <StatCard
              label="Latency (p50 / p95)"
              value={formatDuration(logStats.p50)}
              sub={`avg ${formatDuration(logStats.avg)} · p95 ${formatDuration(logStats.p95)}`}
              icon={<Clock className="h-4 w-4 text-fg-muted" />}
            />
            <StatCard
              label="Tokens (In / Out)"
              value={fmtNum(logStats.totalIn + logStats.totalOut)}
              sub={`${fmtNum(logStats.totalIn)} in → ${fmtNum(logStats.totalOut)} out`}
            />
          </div>

          <Card>
            <CardHeader className="flex-col items-stretch gap-3 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <CardTitle className="flex items-center gap-2">
                  Request Stream
                  <Badge variant="default" className="font-mono text-[10px]">
                    ring buffer · max 2,000
                  </Badge>
                </CardTitle>
                <CardDescription>
                  Click any request row to inspect full headers, token breakdown, and formatted JSON error diagnostics.
                </CardDescription>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <Button variant="default" size="sm" onClick={() => void handleExport("csv")}>
                  <Download className="h-3.5 w-3.5" /> CSV
                </Button>
                <Button variant="default" size="sm" onClick={() => void handleExport("jsonl")}>
                  <Download className="h-3.5 w-3.5" /> JSONL
                </Button>
                {confirmClear ? (
                  <div className="inline-flex items-center gap-1 rounded-lg border border-danger/40 bg-danger/10 p-0.5">
                    <Button variant="danger" size="sm" onClick={() => void clearLogs()}>
                      Confirm Clear
                    </Button>
                    <Button variant="ghost" size="sm" onClick={() => setConfirmClear(false)}>
                      Cancel
                    </Button>
                  </div>
                ) : (
                  <Button variant="danger" size="sm" onClick={() => setConfirmClear(true)}>
                    <Trash2 className="h-3.5 w-3.5" /> Clear
                  </Button>
                )}
              </div>
            </CardHeader>

            <CardContent className="space-y-3">
              {/* Filter Bar */}
              <div className="flex flex-col gap-2.5 lg:flex-row lg:items-center">
                <div className="relative flex-1 min-w-[220px]">
                  <Search className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-fg-muted" />
                  <Input
                    value={filterQuery}
                    onChange={(e) => setFilterQuery(e.target.value)}
                    placeholder="Search model, provider, connection, or error message..."
                    className="pl-8 pr-8 text-xs"
                  />
                  {filterQuery && (
                    <button
                      type="button"
                      onClick={() => setFilterQuery("")}
                      className="absolute top-1/2 right-2 -translate-y-1/2 text-fg-muted hover:text-fg"
                    >
                      <X className="h-3.5 w-3.5" />
                    </button>
                  )}
                </div>

                <div className="flex flex-wrap items-center gap-2">
                  <div className="flex items-center gap-1.5">
                    <Filter className="h-3.5 w-3.5 text-fg-muted" />
                    <Select
                      value={filterProvider}
                      onChange={(e) => setFilterProvider(e.target.value)}
                      className="h-8 text-xs"
                    >
                      <option value="">All providers</option>
                      {providerOptions.map((p) => (
                        <option key={p} value={p}>
                          {p}
                        </option>
                      ))}
                    </Select>
                  </div>

                  <Select
                    value={String(rowLimit)}
                    onChange={(e) => setRowLimit(Number(e.target.value))}
                    className="h-8 text-xs"
                  >
                    <option value="50">Latest 50</option>
                    <option value="150">Latest 150</option>
                    <option value="500">Latest 500</option>
                    <option value="1000">Latest 1,000</option>
                  </Select>

                  {hasActiveFilters && (
                    <Button variant="ghost" size="sm" onClick={resetFilters}>
                      <X className="h-3.5 w-3.5" /> Reset
                    </Button>
                  )}
                </div>
              </div>

              {/* Status Quick Filter Pills */}
              <div className="flex flex-wrap items-center gap-1.5">
                {(
                  [
                    { id: "", label: "All", count: batchCounts.all },
                    { id: "2xx", label: "2xx Success", count: batchCounts.ok },
                    { id: "errors", label: "Errors Only", count: batchCounts.err },
                    { id: "429", label: "429 Quota", count: batchCounts.q429 },
                    { id: "4xx", label: "4xx Client" },
                    { id: "5xx", label: "5xx Server" },
                  ] as Array<{ id: StatusQuickFilter; label: string; count?: number }>
                ).map((pill) => {
                  const active = filterStatus === pill.id;
                  return (
                    <button
                      key={pill.id || "all"}
                      type="button"
                      onClick={() => setFilterStatus(pill.id)}
                      className={cn(
                        "inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] font-medium transition-colors",
                        active
                          ? "border-accent bg-accent/15 text-accent-ink"
                          : "border-border bg-surface-subtle/40 text-fg-muted hover:border-border-strong hover:text-fg"
                      )}
                    >
                      <span>{pill.label}</span>
                      {pill.count !== undefined && (
                        <span className="rounded-full bg-surface px-1.5 py-0.2 font-mono text-[10px]">
                          {pill.count}
                        </span>
                      )}
                    </button>
                  );
                })}
              </div>

              {/* Structured Log Table */}
              {displayedLogs.length === 0 ? (
                <EmptyState
                  title={hasActiveFilters ? "No logs match your current filters" : "No call logs recorded yet"}
                  description={
                    hasActiveFilters
                      ? "Try clearing your search query or switching status/provider filters."
                      : "Inference requests through /v1/* or Playground are automatically logged here."
                  }
                  action={
                    hasActiveFilters ? (
                      <Button variant="default" size="sm" onClick={resetFilters}>
                        Clear Filters
                      </Button>
                    ) : undefined
                  }
                />
              ) : (
                <div className="overflow-x-auto rounded-xl border border-border">
                  <table className="w-full border-collapse text-left text-xs">
                    <thead className="sticky top-0 z-10 border-b border-border bg-surface-subtle/90 text-[11px] uppercase tracking-wider text-fg-muted">
                      <tr>
                        <th className="w-1 p-0" />
                        <th className="px-3 py-2.5 font-semibold">Time</th>
                        <th className="px-3 py-2.5 font-semibold">Status</th>
                        <th className="px-3 py-2.5 font-semibold">Provider &amp; Account</th>
                        <th className="px-3 py-2.5 font-semibold">Model</th>
                        <th className="px-3 py-2.5 font-semibold text-right">Tokens</th>
                        <th className="px-3 py-2.5 font-semibold text-right">Latency</th>
                        <th className="px-3 py-2.5 font-semibold">Diagnostic / Note</th>
                        <th className="w-8 px-2 py-2.5" />
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-border/80">
                      {displayedLogs.map((l) => {
                        const sm = statusMeta(l.status);
                        const isErr = l.status < 200 || l.status >= 400;
                        const readableErr = extractReadableError(l.error_summary);
                        const durPct = Math.min(100, Math.max(6, Math.round((l.duration / 12000) * 100)));
                        const durColor =
                          l.duration < 2500
                            ? "bg-success"
                            : l.duration < 8000
                              ? "bg-warning"
                              : "bg-danger";

                        return (
                          <tr
                            key={l.id}
                            onClick={() => setSelectedLogId(l.id)}
                            className={cn(
                              "group cursor-pointer transition-colors hover:bg-surface-subtle/60",
                              isErr && "bg-danger/[0.025]",
                              selectedLogId === l.id && "bg-accent/10"
                            )}
                          >
                            <td className="p-0 align-stretch">
                              <div className={cn("h-full min-h-[42px] w-1", sm.bar)} />
                            </td>
                            <td
                              className="px-3 py-2.5 whitespace-nowrap"
                              title={formatFullDate(l.timestamp)}
                            >
                              <div className="font-mono text-[11px] font-medium text-fg">
                                {formatClock(l.timestamp)}
                              </div>
                              <div className="text-[10px] text-fg-muted">
                                {formatRelativeTime(l.timestamp)}
                              </div>
                            </td>
                            <td className="px-3 py-2.5 whitespace-nowrap">
                              <div className="flex items-center gap-1.5">
                                <Badge variant={sm.variant} className="font-mono text-[11px] font-semibold">
                                  {l.status}
                                </Badge>
                                <span className="hidden xl:inline font-mono text-[10px] text-fg-muted">
                                  {l.path?.replace("/v1/", "") || "chat"}
                                </span>
                              </div>
                            </td>
                            <td className="px-3 py-2.5 whitespace-nowrap">
                              <div className="flex items-center gap-2">
                                <ProviderIcon providerId={l.provider} size={18} isDark={isDark} />
                                <div className="min-w-0">
                                  <div className="font-medium text-fg">{l.provider}</div>
                                  {l.connection_id && (
                                    <div className="font-mono text-[10px] text-fg-muted">
                                      {l.connection_id}
                                    </div>
                                  )}
                                </div>
                              </div>
                            </td>
                            <td className="px-3 py-2.5 max-w-[240px]">
                              <span
                                className="inline-block max-w-full truncate rounded-md border border-border bg-surface-subtle/60 px-1.5 py-0.5 font-mono text-[11px] text-fg"
                                title={l.model}
                              >
                                {l.model}
                              </span>
                            </td>
                            <td className="px-3 py-2.5 text-right whitespace-nowrap font-mono text-[11px]">
                              {l.tokens_in > 0 || l.tokens_out > 0 ? (
                                <div>
                                  <span className="text-fg">{fmtNum(l.tokens_in)}</span>
                                  <span className="mx-1 text-fg-muted">→</span>
                                  <span className="font-medium text-accent-ink">{fmtNum(l.tokens_out)}</span>
                                </div>
                              ) : (
                                <span className="text-fg-muted">—</span>
                              )}
                            </td>
                            <td className="px-3 py-2.5 text-right whitespace-nowrap">
                              <div className="inline-flex flex-col items-end gap-1">
                                <span className="font-mono text-[11px] font-medium text-fg">
                                  {formatDuration(l.duration)}
                                </span>
                                <div className="h-1 w-14 overflow-hidden rounded-full bg-surface-subtle">
                                  <div className={cn("h-full rounded-full", durColor)} style={{ width: `${durPct}%` }} />
                                </div>
                              </div>
                            </td>
                            <td className="px-3 py-2.5 max-w-[280px]">
                              {readableErr ? (
                                <div
                                  className={cn(
                                    "truncate font-mono text-[11px]",
                                    l.status === 429 ? "text-warning" : "text-danger"
                                  )}
                                  title={readableErr}
                                >
                                  {readableErr}
                                </div>
                              ) : (
                                <span className="text-[11px] text-fg-muted">Completed</span>
                              )}
                            </td>
                            <td className="px-2 py-2.5 text-right text-fg-muted group-hover:text-fg">
                              <ChevronRight className="h-4 w-4" />
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              )}
            </CardContent>
          </Card>
        </div>
      )}

      {/* ═══════════════════ TAB 2: ANALYTICS & CHARTS ═══════════════════ */}
      {activeTab === "analytics" && (
        <div className="space-y-5">
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
              sub={`p50 ${formatDuration(win.p50_latency_ms || 0)} · p95 ${formatDuration(win.p95_latency_ms || 0)}`}
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
                  <div className="w-full overflow-x-auto">
                    <svg
                      viewBox={`0 0 ${Math.max(420, daily.length * 42)} 180`}
                      className="h-52 w-full min-w-[360px]"
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
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>Top Models</CardTitle>
                <Badge variant="default">{byModel.length}</Badge>
              </CardHeader>
              <CardContent>
                {byModel.length === 0 ? (
                  <EmptyState title="No model stats yet" description="Usage breakdown appears here." />
                ) : (
                  <div className="space-y-2.5 max-h-64 overflow-y-auto pr-1">
                    {byModel.slice(0, 12).map((m, i) => (
                      <div
                        key={i}
                        onClick={() => {
                          setFilterProvider(m.provider || "");
                          setFilterQuery(m.model || "");
                          setActiveTab("logs");
                        }}
                        className="flex cursor-pointer items-center justify-between gap-2 rounded-lg border border-border bg-surface-subtle/40 px-2.5 py-2 hover:bg-surface-subtle transition-colors"
                        title="Click to view logs for this model"
                      >
                        <div className="flex items-center gap-2 min-w-0">
                          <ProviderIcon providerId={m.provider} size={18} isDark={isDark} />
                          <div className="min-w-0">
                            <div className="truncate font-mono text-[11px] font-medium text-fg">
                              {m.model}
                            </div>
                            <div className="text-[10px] text-fg-muted">
                              {m.provider} · {formatDuration(m.avg_latency_ms || 0)}
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
        </div>
      )}

      {/* ═══════════════════ TAB 3: BUDGET CAPS ═══════════════════ */}
      {activeTab === "budgets" && (
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

            {budgets.length === 0 ? (
              <EmptyState
                title="No budget caps configured"
                description="Add a global or per-key USD cap above to protect against unexpected spend."
              />
            ) : (
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
                          className={cn("h-full rounded-full transition-all", over ? "bg-danger" : "bg-accent")}
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
      )}

      {/* ═══════════════════ LOG DETAIL INSPECTOR DIALOG ═══════════════════ */}
      <Dialog
        open={!!selectedLog}
        onClose={() => setSelectedLogId(null)}
        title={selectedLog ? `${selectedLog.provider} / ${selectedLog.model}` : "Call Log"}
        description={
          selectedLog
            ? `${selectedLog.method || "POST"} ${selectedLog.path || "/v1/chat/completions"} · ${formatFullDate(selectedLog.timestamp)}`
            : undefined
        }
        icon={
          selectedLog ? (
            <ProviderIcon providerId={selectedLog.provider} size={24} isDark={isDark} />
          ) : undefined
        }
        maxWidth="max-w-2xl"
      >
        {selectedLog && (
          <div className="space-y-4">
            {/* Status + Prev/Next Bar */}
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant={statusMeta(selectedLog.status).variant} className="font-mono text-xs">
                  HTTP {selectedLog.status} · {statusMeta(selectedLog.status).label}
                </Badge>
                <Badge variant="default" className="font-mono text-xs">
                  {formatDuration(selectedLog.duration)}
                </Badge>
                {selectedLog.connection_id && (
                  <Badge variant="default" className="font-mono text-[11px]">
                    acct: {selectedLog.connection_id}
                  </Badge>
                )}
              </div>
              <div className="flex items-center gap-1">
                <Button
                  variant="default"
                  size="sm"
                  disabled={selectedIndex <= 0}
                  onClick={() => setSelectedLogId(displayedLogs[selectedIndex - 1]?.id || null)}
                  title="Newer log"
                >
                  <ChevronLeft className="h-3.5 w-3.5" /> Newer
                </Button>
                <span className="px-1.5 font-mono text-[11px] text-fg-muted">
                  {selectedIndex + 1} / {displayedLogs.length}
                </span>
                <Button
                  variant="default"
                  size="sm"
                  disabled={selectedIndex < 0 || selectedIndex >= displayedLogs.length - 1}
                  onClick={() => setSelectedLogId(displayedLogs[selectedIndex + 1]?.id || null)}
                  title="Older log"
                >
                  Older <ChevronRight className="h-3.5 w-3.5" />
                </Button>
              </div>
            </div>

            {/* Structured Metadata Grid */}
            <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
              <div className="rounded-lg border border-border bg-surface-subtle/40 p-2.5">
                <div className="text-[10px] uppercase tracking-wider text-fg-muted">Input Tokens</div>
                <div className="mt-0.5 font-mono text-sm font-semibold text-fg">
                  {fmtNum(selectedLog.tokens_in || 0)}
                </div>
              </div>
              <div className="rounded-lg border border-border bg-surface-subtle/40 p-2.5">
                <div className="text-[10px] uppercase tracking-wider text-fg-muted">Output Tokens</div>
                <div className="mt-0.5 font-mono text-sm font-semibold text-accent-ink">
                  {fmtNum(selectedLog.tokens_out || 0)}
                </div>
              </div>
              <div className="rounded-lg border border-border bg-surface-subtle/40 p-2.5">
                <div className="text-[10px] uppercase tracking-wider text-fg-muted">Total Tokens</div>
                <div className="mt-0.5 font-mono text-sm font-semibold text-fg">
                  {fmtNum((selectedLog.tokens_in || 0) + (selectedLog.tokens_out || 0))}
                </div>
              </div>
              <div className="rounded-lg border border-border bg-surface-subtle/40 p-2.5">
                <div className="text-[10px] uppercase tracking-wider text-fg-muted">Log ID</div>
                <div className="mt-0.5 truncate font-mono text-[11px] text-fg" title={selectedLog.id}>
                  {selectedLog.id}
                </div>
              </div>
            </div>

            {/* Error Diagnostic Panel (recursively pretty-printed JSON) */}
            {selectedLog.error_summary ? (
              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <div className="text-xs font-semibold text-danger">Error Diagnostic Payload</div>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() =>
                      void handleCopy(
                        "err",
                        formatPrettyError(selectedLog.error_summary),
                        "error payload"
                      )
                    }
                  >
                    {copiedKey === "err" ? (
                      <Check className="h-3.5 w-3.5 text-success" />
                    ) : (
                      <Copy className="h-3.5 w-3.5" />
                    )}
                    Copy Error
                  </Button>
                </div>
                <div className="rounded-lg border border-danger/30 bg-danger/10 px-3 py-2 text-xs font-medium text-danger">
                  {extractReadableError(selectedLog.error_summary)}
                </div>
                <pre className="max-h-64 overflow-auto rounded-xl border border-border bg-surface-subtle/80 p-3 font-mono text-[11px] leading-relaxed text-fg whitespace-pre-wrap break-words">
                  {formatPrettyError(selectedLog.error_summary)}
                </pre>
              </div>
            ) : (
              <div className="flex items-center gap-2 rounded-xl border border-success/30 bg-success/10 px-3.5 py-2.5 text-xs text-success">
                <CheckCircle2 className="h-4 w-4 shrink-0" />
                Request completed cleanly with HTTP {selectedLog.status} in {formatDuration(selectedLog.duration)}.
              </div>
            )}

            {/* Quick Actions Footer */}
            <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border pt-3">
              <div className="flex flex-wrap items-center gap-1.5">
                <Button
                  variant="default"
                  size="sm"
                  onClick={() => {
                    setFilterProvider(selectedLog.provider);
                    setSelectedLogId(null);
                  }}
                >
                  Filter: {selectedLog.provider}
                </Button>
                <Button
                  variant="default"
                  size="sm"
                  onClick={() => {
                    setFilterQuery(selectedLog.model);
                    setSelectedLogId(null);
                  }}
                >
                  Filter Model
                </Button>
              </div>
              <Button
                variant="default"
                size="sm"
                onClick={() =>
                  void handleCopy("json", JSON.stringify(selectedLog, null, 2), "full log JSON")
                }
              >
                {copiedKey === "json" ? (
                  <Check className="h-3.5 w-3.5 text-success" />
                ) : (
                  <Copy className="h-3.5 w-3.5" />
                )}
                Copy Log JSON
              </Button>
            </div>
          </div>
        )}
      </Dialog>
    </div>
  );
}
