import { useEffect, useState } from "react";
import { Search, ArrowRight, KeyRound } from "lucide-react";
import { ALL_NAV_ITEMS, type NavTab } from "./Sidebar";
import { ProviderIcon } from "./ProviderIcon";
import type { OAuthProviderMeta } from "@/lib/api";

export function CommandPalette({
  open,
  onClose,
  onSelectTab,
  oauthProviders,
  onLaunchOAuth,
  isDark,
}: {
  open: boolean;
  onClose: () => void;
  onSelectTab: (tab: NavTab) => void;
  oauthProviders: OAuthProviderMeta[];
  onLaunchOAuth: (providerId: string, displayName: string) => void;
  isDark: boolean;
}) {
  const [query, setQuery] = useState("");

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        if (open) onClose();
      } else if (e.key === "Escape" && open) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  useEffect(() => {
    if (open) setQuery("");
  }, [open]);

  if (!open) return null;
  const q = query.trim().toLowerCase();

  const matchedNav = ALL_NAV_ITEMS.filter(
    (i) =>
      !q ||
      i.label.toLowerCase().includes(q) ||
      i.description.toLowerCase().includes(q)
  );
  const matchedOAuth = oauthProviders
    .filter(
      (p) =>
        !q ||
        p.id.toLowerCase().includes(q) ||
        p.display_name.toLowerCase().includes(q)
    )
    .slice(0, 6);

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 backdrop-blur-[2px] pt-[14vh] p-4"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="w-full max-w-xl overflow-hidden rounded-2xl border border-border-strong bg-surface shadow-2xl">
        <div className="flex items-center gap-3 border-b border-border px-4 py-3">
          <Search className="h-4 w-4 text-fg-muted" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Jump to page or launch OAuth login (e.g. antigravity, claude, codex)..."
            className="flex-1 bg-transparent text-sm text-fg placeholder:text-fg-muted/70 focus:outline-none"
          />
          <kbd className="rounded border border-border bg-surface-subtle px-1.5 py-0.5 font-mono text-[10px] text-fg-muted">
            ESC
          </kbd>
        </div>

        <div className="max-h-[55vh] overflow-y-auto p-2 space-y-3">
          {matchedNav.length > 0 && (
            <div>
              <div className="px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wider text-fg-muted">
                Pages
              </div>
              {matchedNav.map((item) => {
                const Icon = item.icon;
                return (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => {
                      onSelectTab(item.id);
                      onClose();
                    }}
                    className="flex w-full items-center gap-3 rounded-lg px-2.5 py-2 text-left text-sm hover:bg-surface-subtle cursor-pointer"
                  >
                    <Icon className="h-4 w-4 text-fg-muted" />
                    <div className="min-w-0 flex-1">
                      <div className="font-medium text-fg">{item.label}</div>
                      <div className="truncate text-xs text-fg-muted">{item.description}</div>
                    </div>
                    <ArrowRight className="h-3.5 w-3.5 text-fg-muted" />
                  </button>
                );
              })}
            </div>
          )}

          {matchedOAuth.length > 0 && (
            <div>
              <div className="px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wider text-fg-muted">
                Quick OAuth Login
              </div>
              {matchedOAuth.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  onClick={() => {
                    onClose();
                    onLaunchOAuth(p.id, p.display_name);
                  }}
                  className="flex w-full items-center gap-3 rounded-lg px-2.5 py-2 text-left text-sm hover:bg-surface-subtle cursor-pointer"
                >
                  <ProviderIcon providerId={p.id} size={20} isDark={isDark} />
                  <div className="min-w-0 flex-1">
                    <div className="font-medium text-fg">Login with {p.display_name}</div>
                    <div className="font-mono text-[11px] text-fg-muted">{p.id} · {p.flow}</div>
                  </div>
                  <KeyRound className="h-3.5 w-3.5 text-accent-ink" />
                </button>
              ))}
            </div>
          )}

          {!matchedNav.length && !matchedOAuth.length && (
            <div className="py-8 text-center text-xs text-fg-muted">
              No matching pages or OAuth providers.
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
