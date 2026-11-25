import { useState } from "react";
import {
  LayoutDashboard,
  MessageSquareText,
  Layers,
  GitBranch,
  KeyRound,
  ShieldCheck,
  Sparkles,
  Network,
  Bot,
  BrainCircuit,
  Database,
  Activity,
  Gauge,
  Terminal,
  Search,
  ChevronDown,
  ChevronRight,
  PanelLeftClose,
  PanelLeftOpen,
  X,
} from "lucide-react";
import { OmniRouteLogo } from "./OmniRouteLogo";
import { cn, fmtUptime } from "@/lib/utils";
import type { GatewayHealth } from "@/lib/api";

export type NavTab =
  | "overview"
  | "providers"
  | "playground"
  | "combos"
  | "oauth"
  | "quota"
  | "analytics"
  | "cli"
  | "security"
  | "optimization"
  | "mitm"
  | "a2a"
  | "memory"
  | "database";

export interface NavItemDef {
  id: NavTab;
  label: string;
  description: string;
  icon: React.ComponentType<{ className?: string }>;
  badgeKey?: "providers" | "combos" | "oauth";
}

export const NAV_SECTIONS: Array<{
  id: string;
  title: string;
  items: NavItemDef[];
}> = [
  {
    id: "gateway",
    title: "Gateway",
    items: [
      { id: "overview", label: "Overview", description: "Health, metrics & quick-connect SDKs", icon: LayoutDashboard },
      { id: "providers", label: "Providers", description: "394 providers, OAuth logins & API keys", icon: Layers, badgeKey: "providers" },
      { id: "playground", label: "Playground", description: "Live multi-provider chat tester", icon: MessageSquareText },
      { id: "oauth", label: "OAuth & Tokens", description: "PKCE, Device Code & auto refresh", icon: KeyRound, badgeKey: "oauth" },
    ],
  },
  {
    id: "routing",
    title: "Routing & Spend",
    items: [
      { id: "combos", label: "Combos & Routing", description: "22 strategies & dry-run simulator", icon: GitBranch, badgeKey: "combos" },
      { id: "quota", label: "Quota & Proxies", description: "Live quota bars, model sync & SOCKS5", icon: Gauge },
      { id: "analytics", label: "Logs & Analytics", description: "Daily charts, call logs & budget caps", icon: Activity },
      { id: "cli", label: "Aliases & CLI Tools", description: "Model aliases, IDE setup & OpenAPI", icon: Terminal },
      { id: "optimization", label: "Compression & Cache", description: "RTK, Caveman & semantic cache", icon: Sparkles },
      { id: "security", label: "Security & Alerts", description: "API keys, login lock, guardrails & alerts", icon: ShieldCheck },
    ],
  },
  {
    id: "agents",
    title: "Protocols & Agents",
    items: [
      { id: "mitm", label: "MITM TLS Proxy", description: "ECDSA P-256 live HTTPS interception", icon: Network },
      { id: "a2a", label: "A2A Protocol", description: "Agent-to-Agent v0.3 / v1.0 JSON-RPC", icon: Bot },
      { id: "memory", label: "Memory & Skills", description: "TF-cosine agent memory & skills", icon: BrainCircuit },
    ],
  },
  {
    id: "system",
    title: "System",
    items: [
      { id: "database", label: "Database & Migration", description: "SQLite WAL, hot backup & Node import", icon: Database },
    ],
  },
];

export const ALL_NAV_ITEMS: NavItemDef[] = NAV_SECTIONS.flatMap((s) => s.items);

export function Sidebar({
  activeTab,
  onSelect,
  collapsed,
  onToggleCollapse,
  mobileOpen,
  onCloseMobile,
  health,
  oauthCount,
}: {
  activeTab: NavTab;
  onSelect: (t: NavTab) => void;
  collapsed: boolean;
  onToggleCollapse: () => void;
  mobileOpen: boolean;
  onCloseMobile: () => void;
  health: GatewayHealth | null;
  oauthCount: number;
}) {
  const [filter, setFilter] = useState("");
  const [closedGroups, setClosedGroups] = useState<Record<string, boolean>>({});

  const getBadge = (key?: NavItemDef["badgeKey"]): string | null => {
    if (key === "providers" && health) return String(health.provider_count);
    if (key === "combos" && health) return String(health.combos_count);
    if (key === "oauth" && oauthCount > 0) return String(oauthCount);
    return null;
  };

  const toggleGroup = (id: string) =>
    setClosedGroups((prev) => ({ ...prev, [id]: !prev[id] }));

  const q = filter.trim().toLowerCase();

  const inner = (
    <div className="flex h-full flex-col bg-bg">
      {/* Brand Lockup */}
      <div className="flex h-14 shrink-0 items-center justify-between border-b border-border px-3.5">
        <button
          type="button"
          onClick={() => {
            onSelect("overview");
            onCloseMobile();
          }}
          className="flex items-center gap-2.5 overflow-hidden text-left cursor-pointer"
        >
          <OmniRouteLogo size={28} />
          {!collapsed && (
            <div className="min-w-0">
              <div className="flex items-center gap-1.5">
                <span className="truncate text-sm font-semibold tracking-tight text-fg">
                  OmniRoute
                </span>
                <span className="rounded bg-accent-subtle px-1.5 py-0.5 font-mono text-[10px] font-semibold text-accent-ink">
                  RS
                </span>
              </div>
              <div className="truncate font-mono text-[10px] text-fg-muted">
                v{health?.upstream_sync_version || "3.8.55"} · native
              </div>
            </div>
          )}
        </button>

        <button
          type="button"
          onClick={onToggleCollapse}
          className="hidden h-7 w-7 items-center justify-center rounded-md text-fg-muted hover:bg-surface-subtle hover:text-fg lg:inline-flex cursor-pointer"
          title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
        >
          {collapsed ? <PanelLeftOpen className="h-4 w-4" /> : <PanelLeftClose className="h-4 w-4" />}
        </button>

        <button
          type="button"
          onClick={onCloseMobile}
          className="inline-flex h-7 w-7 items-center justify-center rounded-md text-fg-muted hover:bg-surface-subtle hover:text-fg lg:hidden cursor-pointer"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      {/* Inline Sidebar Filter */}
      {!collapsed && (
        <div className="px-3 pt-3 pb-1">
          <div className="relative">
            <Search className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-fg-muted" />
            <input
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Filter navigation..."
              className="h-8 w-full rounded-lg border border-border bg-surface pl-8 pr-2.5 text-xs text-fg placeholder:text-fg-muted/70 focus:border-ring focus:outline-none"
            />
          </div>
        </div>
      )}

      {/* Navigation Sections */}
      <nav className="flex-1 overflow-y-auto px-2.5 py-2 space-y-3">
        {NAV_SECTIONS.map((section) => {
          const visibleItems = section.items.filter(
            (i) =>
              !q ||
              i.label.toLowerCase().includes(q) ||
              i.description.toLowerCase().includes(q)
          );
          if (!visibleItems.length) return null;
          const isGroupClosed = !q && !!closedGroups[section.id];

          return (
            <div key={section.id}>
              {!collapsed && (
                <button
                  type="button"
                  onClick={() => toggleGroup(section.id)}
                  className="mb-1 flex w-full items-center justify-between px-2 py-1 text-[11px] font-semibold uppercase tracking-wider text-fg-muted hover:text-fg cursor-pointer"
                >
                  <span>{section.title}</span>
                  {isGroupClosed ? (
                    <ChevronRight className="h-3 w-3" />
                  ) : (
                    <ChevronDown className="h-3 w-3" />
                  )}
                </button>
              )}

              {!isGroupClosed && (
                <div className="space-y-0.5">
                  {visibleItems.map((item) => {
                    const Icon = item.icon;
                    const active = activeTab === item.id;
                    const badge = getBadge(item.badgeKey);
                    return (
                      <button
                        key={item.id}
                        type="button"
                        onClick={() => {
                          onSelect(item.id);
                          onCloseMobile();
                        }}
                        title={collapsed ? item.label : undefined}
                        className={cn(
                          "group flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-[13px] font-medium transition-all cursor-pointer",
                          collapsed && "justify-center px-0",
                          active
                            ? "sidebar-active font-semibold"
                            : "text-fg-muted hover:bg-surface-subtle hover:text-fg"
                        )}
                      >
                        <Icon className="h-4 w-4 shrink-0" />
                        {!collapsed && (
                          <>
                            <span className="flex-1 truncate text-left">{item.label}</span>
                            {badge && (
                              <span
                                className={cn(
                                  "rounded-md px-1.5 py-0.5 font-mono text-[10px]",
                                  active
                                    ? "bg-white/15 text-current"
                                    : "bg-surface-subtle text-fg-muted border border-border"
                                )}
                              >
                                {badge}
                              </span>
                            )}
                          </>
                        )}
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          );
        })}
      </nav>

      {/* Gateway Live Status Footer */}
      <div className="shrink-0 border-t border-border p-3">
        {collapsed ? (
          <div className="flex justify-center" title="Gateway Online">
            <span className="h-2.5 w-2.5 rounded-full bg-success ring-4 ring-success/20" />
          </div>
        ) : (
          <div className="flex items-center justify-between rounded-lg border border-border bg-surface px-3 py-2">
            <div className="flex items-center gap-2 min-w-0">
              <span
                className={cn(
                  "h-2 w-2 shrink-0 rounded-full",
                  health ? "bg-success ring-4 ring-success/20" : "bg-warning"
                )}
              />
              <span className="truncate text-xs font-medium text-fg">
                {health ? "Gateway live" : "Connecting..."}
              </span>
            </div>
            <span className="font-mono text-[11px] text-fg-muted">
              {health ? fmtUptime(health.uptime_secs) : "—"}
            </span>
          </div>
        )}
      </div>
    </div>
  );

  return (
    <>
      {/* Desktop Rail */}
      <aside
        className={cn(
          "hidden shrink-0 border-r border-border transition-all duration-200 lg:block",
          collapsed ? "w-16" : "w-64"
        )}
      >
        {inner}
      </aside>

      {/* Mobile Drawer */}
      {mobileOpen && (
        <div className="fixed inset-0 z-50 flex lg:hidden">
          <div
            className="fixed inset-0 bg-black/60 backdrop-blur-[1px]"
            onClick={onCloseMobile}
          />
          <div className="relative z-10 w-68 max-w-[85vw] border-r border-border shadow-2xl">
            {inner}
          </div>
        </div>
      )}
    </>
  );
}
