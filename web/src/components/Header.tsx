import {
  Menu,
  Search,
  RefreshCw,
  Sun,
  Moon,
  Palette,
  KeyRound,
  CheckCircle2,
  LogOut,
  Lock,
  Unlock,
} from "lucide-react";
import { Button, Badge } from "./ui/primitives";
import { ALL_NAV_ITEMS, type NavTab } from "./Sidebar";
import { cn } from "@/lib/utils";

export function Header({
  activeTab,
  theme,
  appearance,
  hasApiKey,
  keysCount = 0,
  dashboardLocked,
  refreshing,
  onOpenMobile,
  onOpenPalette,
  onToggleTheme,
  onToggleAppearance,
  onRefresh,
  onOpenApiKey,
  onOpenSecurity,
  onLogout,
}: {
  activeTab: NavTab;
  theme: "dark" | "light";
  appearance: "editions" | "classic";
  hasApiKey: boolean;
  keysCount?: number;
  dashboardLocked?: boolean;
  refreshing: boolean;
  onOpenMobile: () => void;
  onOpenPalette: () => void;
  onToggleTheme: () => void;
  onToggleAppearance: () => void;
  onRefresh: () => void;
  onOpenApiKey: () => void;
  onOpenSecurity?: () => void;
  onLogout?: () => void;
}) {
  const current = ALL_NAV_ITEMS.find((i) => i.id === activeTab);

  return (
    <header className="sticky top-0 z-30 flex h-14 shrink-0 items-center justify-between gap-3 border-b border-border bg-bg/85 px-4 backdrop-blur-md lg:px-6">
      <div className="flex items-center gap-3 min-w-0">
        <Button
          variant="ghost"
          size="icon"
          onClick={onOpenMobile}
          className="lg:hidden"
          aria-label="Open menu"
        >
          <Menu className="h-4 w-4" />
        </Button>

        <div className="flex items-center gap-2 min-w-0">
          <span className="truncate text-sm font-semibold text-fg">
            {current?.label || "Overview"}
          </span>
          <span className="hidden text-xs text-fg-muted xl:inline">
            — {current?.description}
          </span>
        </div>
      </div>

      <div className="flex items-center gap-2">
        {/* Command Palette Trigger (Ctrl+K) */}
        <button
          type="button"
          onClick={onOpenPalette}
          className="hidden h-8 items-center gap-2 rounded-lg border border-border-strong bg-surface px-2.5 text-xs text-fg-muted transition-colors hover:border-fg-muted hover:text-fg sm:inline-flex cursor-pointer"
        >
          <Search className="h-3.5 w-3.5" />
          <span className="pr-3">Quick search...</span>
          <kbd className="rounded border border-border bg-surface-subtle px-1.5 py-0.5 font-mono text-[10px] text-fg-muted">
            Ctrl K
          </kbd>
        </button>

        {/* Editions / Classic Palette Switcher */}
        <Button
          variant="ghost"
          size="sm"
          onClick={onToggleAppearance}
          title="Switch palette: Editions (Warm Copper) / Classic (Acid Lime)"
          className="hidden md:inline-flex"
        >
          <Palette className="h-3.5 w-3.5 text-accent-ink" />
          <span className="capitalize">{appearance}</span>
        </Button>

        {/* Light / Dark Mode Switcher */}
        <Button
          variant="ghost"
          size="icon"
          onClick={onToggleTheme}
          title={theme === "dark" ? "Switch to Light Mode" : "Switch to Dark Mode"}
        >
          {theme === "dark" ? (
            <Sun className="h-4 w-4" />
          ) : (
            <Moon className="h-4 w-4" />
          )}
        </Button>

        {/* Refresh */}
        <Button
          variant="ghost"
          size="icon"
          onClick={onRefresh}
          title="Refresh gateway telemetry"
        >
          <RefreshCw className={cn("h-4 w-4", refreshing && "animate-spin")} />
        </Button>

        {/* Quick Connect & API Keys Modal Trigger */}
        <Button
          variant={hasApiKey || keysCount > 0 ? "primary" : "default"}
          size="sm"
          onClick={onOpenApiKey}
          title="Generate, copy & manage Gateway API keys and Base URLs"
        >
          {hasApiKey ? (
            <CheckCircle2 className="h-3.5 w-3.5" />
          ) : (
            <KeyRound className="h-3.5 w-3.5" />
          )}
          <span className="hidden sm:inline">
            {keysCount > 0 ? `Connect · ${keysCount} Key${keysCount > 1 ? "s" : ""}` : "Connect / Keys"}
          </span>
        </Button>

        {dashboardLocked ? (
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={onOpenSecurity}
              title="Dashboard is password-protected (click to manage)"
              className="hidden h-7 items-center gap-1 rounded-full border border-success/35 bg-success/10 px-2.5 text-[11px] font-medium text-success transition-colors hover:bg-success/15 md:inline-flex cursor-pointer"
            >
              <Lock className="h-3 w-3" />
              Locked
            </button>
            {onLogout && (
              <Button variant="ghost" size="icon" onClick={onLogout} title="Sign out of dashboard">
                <LogOut className="h-4 w-4 text-fg-muted" />
              </Button>
            )}
          </div>
        ) : (
          onOpenSecurity && (
            <button
              type="button"
              onClick={onOpenSecurity}
              title="Dashboard UI is unlocked — click to set a password"
              className="hidden h-7 items-center gap-1 rounded-full border border-warning/40 bg-warning/10 px-2.5 text-[11px] font-medium text-warning transition-colors hover:bg-warning/15 md:inline-flex cursor-pointer"
            >
              <Unlock className="h-3 w-3" />
              Unlocked
            </button>
          )
        )}

        <Badge variant="success" className="hidden xl:inline-flex font-mono">
          Rust Native
        </Badge>
      </div>
    </header>
  );
}
