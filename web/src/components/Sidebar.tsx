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

  const renderInner = (isCollapsed: boolean) => (
    <div className="flex h-full flex-col bg-bg">
      {/* Brand Lockup */}
      <div
        className={cn(
          "flex h-14 shrink-0 items-center border-b border-border",
          isCollapsed ? "justify-center px-0" : "justify-between px-3.5"
        )}
      >
        <button
          type="button"
          onClick={() => {
            if (isCollapsed) {
              onToggleCollapse();
            } else {
              onSelect("overview");
              onCloseMobile();
            }
          }}
          title={isCollapsed ? "Expand sidebar" : "OmniRoute RS"}
          className={cn(
            "flex items-center overflow-hidden text-left cursor-pointer",
            isCollapsed
              ? "h-10 w-10 justify-center rounded-xl hover:bg-surface-subtle transition-colors"
              : "gap-2.5"
          )}
        >
          <OmniRouteLogo size={28} />
          {!isCollapsed && (
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

        {!isCollapsed && (
          <button
            type="button"
            onClick={onToggleCollapse}
            className="hidden h-7 w-7 items-center justify-center rounded-md text-fg-muted hover:bg-surface-subtle hover:text-fg lg:inline-flex cursor-pointer"
            title="Collapse sidebar"
          >
            <PanelLeftClose className="h-4 w-4" />
          </button>
        )}

        {!isCollapsed && (
          <button
            type="button"
            onClick={onCloseMobile}
            className="inline-flex h-7 w-7 items-center justify-center rounded-md text-fg-muted hover:bg-surface-subtle hover:text-fg lg:hidden cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        )}
      </div>

      {/* Inline Sidebar Filter */}
      {!isCollapsed && (
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
      <nav
        className={cn(
          "flex-1 overflow-y-auto py-2.5",
          isCollapsed ? "px-0 space-y-1" : "px-2.5 space-y-3"
        )}
      >
        {NAV_SECTIONS.map((section, sectionIdx) => {
          const visibleItems = section.items.filter(
            (i) =>
              isCollapsed ||
              !q ||
              i.label.toLowerCase().includes(q) ||
              i.description.toLowerCase().includes(q)
          );
          if (!visibleItems.length) return null;
          const isGroupClosed = !isCollapsed && !q && !!closedGroups[section.id];

          return (
            <div key={section.id}>
              {isCollapsed ? (
                sectionIdx > 0 && <div className="mx-auto my-2 h-px w-6 bg-border" />
              ) : (
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
                <div className={cn(isCollapsed ? "flex flex-col items-center gap-1" : "space-y-0.5")}>
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
                        title={isCollapsed ? `${item.label} — ${item.description}` : undefined}
                        className={cn(
                          "group flex items-center text-[13px] font-medium transition-all cursor-pointer",
                          isCollapsed
                            ? "h-10 w-10 justify-center rounded-xl p-0"
                            : "w-full gap-2.5 rounded-lg px-2.5 py-2",
                          active
                            ? "sidebar-active font-semibold"
                            : "text-fg-muted hover:bg-surface-subtle hover:text-fg"
                        )}
                      >
                        <Icon className="h-[18px] w-[18px] shrink-0" />
                        {!isCollapsed && (
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
      <div className={cn("shrink-0 border-t border-border", isCollapsed ? "py-2.5 px-0" : "p-3")}>
        {isCollapsed ? (
          <div className="flex flex-col items-center gap-2.5">
            <button
              type="button"
              onClick={onToggleCollapse}
              className="inline-flex h-9 w-9 items-center justify-center rounded-xl text-fg-muted hover:bg-surface-subtle hover:text-fg transition-colors cursor-pointer"
              title="Expand sidebar"
            >
              <PanelLeftOpen className="h-4 w-4" />
            </button>
            <div
              className="flex h-6 w-6 items-center justify-center"
              title={health ? `Gateway live (${fmtUptime(health.uptime_secs)})` : "Connecting..."}
            >
              <span
                className={cn(
                  "h-2.5 w-2.5 rounded-full",
                  health ? "bg-success ring-4 ring-success/20" : "bg-warning"
                )}
              />
            </div>
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
        {renderInner(collapsed)}
      </aside>

      {/* Mobile Drawer */}
      {mobileOpen && (
        <div className="fixed inset-0 z-50 flex lg:hidden">
          <div
            className="fixed inset-0 bg-black/60 backdrop-blur-[1px]"
            onClick={onCloseMobile}
          />
          <div className="relative z-10 w-68 max-w-[85vw] border-r border-border shadow-2xl">
            {renderInner(false)}
          </div>
        </div>
      )}
    </>
  );
}
