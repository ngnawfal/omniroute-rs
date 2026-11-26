import { useCallback, useEffect, useRef, useState } from "react";
import { Eye, EyeOff, Lock, ShieldCheck } from "lucide-react";
import { Sidebar, type NavTab } from "./components/Sidebar";
import { Header } from "./components/Header";
import { CommandPalette } from "./components/CommandPalette";
import {
  OAuthModal,
  openOAuthPopupSync,
  type OAuthLaunchRequest,
} from "./components/OAuthModal";
import { ApiKeyModal, CredentialModal } from "./components/CredentialModal";
import { ToastStack, type ToastItem } from "./components/Toast";
import { OverviewPage } from "./pages/OverviewPage";
import { ProvidersPage } from "./pages/ProvidersPage";
import { PlaygroundPage } from "./pages/PlaygroundPage";
import { CombosPage } from "./pages/CombosPage";
import { OAuthPage } from "./pages/OAuthPage";
import { SecurityPage } from "./pages/SecurityPage";
import { OptimizationPage } from "./pages/OptimizationPage";
import { MitmPage } from "./pages/MitmPage";
import { A2APage } from "./pages/A2APage";
import { MemorySkillsPage } from "./pages/MemorySkillsPage";
import { DatabasePage } from "./pages/DatabasePage";
import { AnalyticsLogsPage } from "./pages/AnalyticsLogsPage";
import { QuotaProxyPage } from "./pages/QuotaProxyPage";
import { AliasesCliPage } from "./pages/AliasesCliPage";
import { OmniRouteLogo } from "./components/OmniRouteLogo";
import { Button, Input } from "./components/ui/primitives";
import {
  api,
  apiPost,
  getStoredApiKey,
  setStoredApiKey,
  type ComboItem,
  type CredentialItem,
  type GamificationStatus,
  type GatewayHealth,
  type OAuthProviderMeta,
} from "./lib/api";

const VALID_TABS: readonly NavTab[] = [
  "overview",
  "providers",
  "playground",
  "combos",
  "oauth",
  "quota",
  "analytics",
  "cli",
  "security",
  "optimization",
  "mitm",
  "a2a",
  "memory",
  "database",
];

function parseNavTab(raw: string | null | undefined): NavTab | null {
  if (!raw) return null;
  const clean = raw.replace(/^#\/?/, "").trim().toLowerCase();
  return (VALID_TABS as readonly string[]).includes(clean) ? (clean as NavTab) : null;
}

function getInitialTab(): NavTab {
  try {
    const fromHash = parseNavTab(window.location.hash);
    if (fromHash) return fromHash;
    const fromStorage = parseNavTab(localStorage.getItem("omniroute_active_tab"));
    if (fromStorage) return fromStorage;
  } catch {
    // ignore storage/window errors
  }
  return "overview";
}

export default function App() {
  const [activeTab, setActiveTab] = useState<NavTab>(getInitialTab);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    try {
      return localStorage.getItem("omniroute_sidebar_collapsed") === "1";
    } catch {
      return false;
    }
  });
  const [mobileOpen, setMobileOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [apiKeyOpen, setApiKeyOpen] = useState(false);
  const [hasApiKey, setHasApiKey] = useState(() => !!getStoredApiKey());

  const [theme, setTheme] = useState<"dark" | "light">(() => {
    try {
      const saved = localStorage.getItem("omniroute_theme");
      return saved === "light" ? "light" : "dark";
    } catch {
      return "dark";
    }
  });
  const [appearance, setAppearance] = useState<"editions" | "classic">(() => {
    try {
      const saved = localStorage.getItem("omniroute_appearance");
      return saved === "classic" ? "classic" : "editions";
    } catch {
      return "editions";
    }
  });

  // Live backend state
  const [refreshing, setRefreshing] = useState(false);
  const [health, setHealth] = useState<GatewayHealth | null>(null);
  const [gamification, setGamification] = useState<GamificationStatus | null>(null);
  const [credentials, setCredentials] = useState<CredentialItem[]>([]);
  const [oauthProviders, setOAuthProviders] = useState<OAuthProviderMeta[]>([]);
  const [oauthStatus, setOAuthStatus] = useState<GatewayHealth["oauth"] | undefined>(undefined);
  const [combos, setCombos] = useState<ComboItem[]>([]);
  const [strategies, setStrategies] = useState<string[]>([]);
  const [keys, setKeys] = useState<any[]>([]);
  const [guardrails, setGuardrails] = useState<any>(null);
  const [mitm, setMitm] = useState<any>(null);
  const [a2aTasks, setA2ATasks] = useState<any[]>([]);
  const [memories, setMemories] = useState<any[]>([]);
  const [skills, setSkills] = useState<any[]>([]);
  const [db, setDb] = useState<any>(null);
  const [authChecked, setAuthChecked] = useState(false);
  const [authState, setAuthState] = useState<{ locked: boolean; authenticated: boolean }>({
    locked: false,
    authenticated: false,
  });
  const [loginPassword, setLoginPassword] = useState("");
  const [showLoginPw, setShowLoginPw] = useState(false);
  const [loginError, setLoginError] = useState("");
  const [loggingIn, setLoggingIn] = useState(false);

  // Modals
  const [oauthReq, setOAuthReq] = useState<OAuthLaunchRequest | null>(null);
  const [credModal, setCredModal] = useState<{ open: boolean; provider: string }>({
    open: false,
    provider: "",
  });
  const [playgroundTarget, setPlaygroundTarget] = useState<{
    provider: string;
    model: string;
  } | null>(null);

  // Toasts
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const toastSeq = useRef(1);
  const addToast = useCallback(
    (message: string, tone: "success" | "error" | "info" = "success") => {
      const id = toastSeq.current++;
      setToasts((prev) => [...prev.slice(-4), { id, message, tone }]);
      window.setTimeout(() => {
        setToasts((prev) => prev.filter((t) => t.id !== id));
      }, 3600);
    },
    []
  );

  // Sync theme & appearance onto <html>
  useEffect(() => {
    const el = document.documentElement;
    el.setAttribute("data-theme", theme);
    el.setAttribute("data-ci-appearance", appearance);
    el.classList.toggle("dark", theme === "dark");
    try {
      localStorage.setItem("omniroute_theme", theme);
      localStorage.setItem("omniroute_appearance", appearance);
    } catch {}
  }, [theme, appearance]);

  // Persist active tab to URL hash + localStorage so refresh & Back/Forward preserve current page
  useEffect(() => {
    try {
      localStorage.setItem("omniroute_active_tab", activeTab);
      const targetHash = `#${activeTab}`;
      if (window.location.hash !== targetHash) {
        window.history.replaceState(null, "", targetHash);
      }
    } catch {}
  }, [activeTab]);

  useEffect(() => {
    const onHashChange = () => {
      const next = parseNavTab(window.location.hash);
      if (next) setActiveTab(next);
    };
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  useEffect(() => {
    try {
      localStorage.setItem("omniroute_sidebar_collapsed", sidebarCollapsed ? "1" : "0");
    } catch {}
  }, [sidebarCollapsed]);

  // Global Ctrl+K shortcut for Command Palette
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const checkAuth = useCallback(async () => {
    const r = await api<any>("/api/auth/status");
    if (r.ok && r.data) {
      const next = {
        locked: !!r.data.locked,
        authenticated: !!r.data.authenticated,
      };
      setAuthState(next);
      setAuthChecked(true);
      return next;
    }
    setAuthChecked(true);
    return { locked: false, authenticated: true };
  }, []);

  const refreshAll = useCallback(async () => {
    setRefreshing(true);
    setHasApiKey(!!getStoredApiKey());
    const auth = await checkAuth();
    if (auth.locked && !auth.authenticated) {
      setRefreshing(false);
      return;
    }
    const [h, g, m, gr, dbRes, memRes, skRes, oaRes, keysRes, credsRes, combosRes, tasksRes] =
      await Promise.all([
        api<GatewayHealth>("/api/health"),
        api<GamificationStatus>("/api/gamification/status"),
        api<any>("/api/mitm/status"),
        api<any>("/api/guardrails/status"),
        api<any>("/api/db/status"),
        api<any>("/api/memory"),
        api<any>("/api/skills"),
        api<any>("/api/oauth/providers"),
        api<any>("/api/keys"),
        api<any>("/api/providers/credentials"),
        api<any>("/api/combos"),
        api<any>("/api/a2a/tasks"),
      ]);
    setRefreshing(false);

    if (h.ok) setHealth(h.data);
    if (g.ok) setGamification(g.data);
    if (m.ok) setMitm(m.data);
    if (gr.ok) setGuardrails(gr.data);
    if (dbRes.ok) setDb(dbRes.data);
    if (memRes.ok) setMemories(memRes.data?.data || memRes.data?.memories || []);
    if (skRes.ok) setSkills(skRes.data?.skills || []);
    if (oaRes.ok) {
      setOAuthProviders(oaRes.data?.providers || []);
      setOAuthStatus(oaRes.data?.status);
    }
    if (keysRes.ok) {
      const list = keysRes.data?.keys || [];
      setKeys(list);
      // Auto-sync first active gateway API key into browser localStorage so Playground
      // & /v1/* tools work immediately even when a key was created from CLI / another browser.
      if (!getStoredApiKey() && list.length > 0) {
        const active = list.find((k: any) => !k.disabled && k.key) || list[0];
        if (active?.key) {
          setStoredApiKey(active.key);
          setHasApiKey(true);
        }
      }
    }
    if (credsRes.ok) setCredentials(credsRes.data?.credentials || []);
    if (combosRes.ok) {
      setCombos(combosRes.data?.combos || []);
      setStrategies(combosRes.data?.strategies || []);
    }
    if (tasksRes.ok) setA2ATasks(tasksRes.data?.tasks || []);
  }, [checkAuth]);

  useEffect(() => {
    void refreshAll();
  }, [refreshAll]);

  // Synchronously open popup inside click event so browser never blocks OAuth redirect!
  const launchOAuth = useCallback(
    (providerId: string, displayName: string) => {
      const meta = oauthProviders.find((p) => p.id === providerId);
      const popup =
        meta?.flow === "import_only" ? null : openOAuthPopupSync(displayName || providerId);
      setOAuthReq({ providerId, displayName: displayName || providerId, popup });
    },
    [oauthProviders]
  );

  const isDark = theme === "dark";

  const handleDashboardLogin = async () => {
    setLoggingIn(true);
    setLoginError("");
    const r = await apiPost("/api/auth/login", { password: loginPassword });
    setLoggingIn(false);
    if (r.ok) {
      setLoginPassword("");
      setAuthState({ locked: true, authenticated: true });
      void refreshAll();
    } else {
      setLoginError(r.error || "Invalid password");
    }
  };

  const handleDashboardLogout = async () => {
    await apiPost("/api/auth/logout", {});
    // Clear browser-cached API key & sensitive in-memory telemetry on logout
    setStoredApiKey("");
    setHasApiKey(false);
    setHealth(null);
    setKeys([]);
    setCredentials([]);
    setCombos([]);
    setMemories([]);
    setSkills([]);
    setA2ATasks([]);
    setDb(null);
    setGuardrails(null);
    setMitm(null);
    setAuthState((prev) => ({ ...prev, authenticated: false }));
    addToast("Signed out of dashboard", "info");
  };

  if (!authChecked) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-bg p-4 text-fg">
        <div className="flex flex-col items-center gap-3 text-center">
          <OmniRouteLogo size={42} />
          <div className="h-5 w-5 animate-spin rounded-full border-2 border-border-strong border-t-[var(--accent)]" />
          <span className="text-xs text-fg-muted">Checking gateway session...</span>
        </div>
      </div>
    );
  }

  if (authState.locked && !authState.authenticated) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-bg p-4 text-fg">
        <div className="w-full max-w-sm rounded-2xl border border-border-strong bg-surface p-6 shadow-2xl space-y-5">
          <div className="flex flex-col items-center text-center space-y-2.5">
            <OmniRouteLogo size={44} />
            <div className="inline-flex items-center gap-1.5 rounded-full border border-success/35 bg-success/10 px-2.5 py-0.5 text-[11px] font-medium text-success">
              <ShieldCheck className="h-3.5 w-3.5" />
              Protected Gateway
            </div>
            <h1 className="text-lg font-semibold tracking-tight text-fg">OmniRoute RS</h1>
            <p className="text-xs text-fg-muted leading-relaxed">
              Dashboard ini dilindungi password. Masuk dengan password admin untuk mengelola provider, combo, dan log.
            </p>
          </div>
          <div className="space-y-3">
            <div className="relative">
              <Input
                type={showLoginPw ? "text" : "password"}
                autoFocus
                autoComplete="current-password"
                value={loginPassword}
                onChange={(e) => setLoginPassword(e.target.value)}
                placeholder="Dashboard password"
                className="pr-10"
                onKeyDown={(e) => {
                  if (e.key === "Enter" && loginPassword.trim()) void handleDashboardLogin();
                }}
              />
              <button
                type="button"
                tabIndex={-1}
                onClick={() => setShowLoginPw((v) => !v)}
                aria-label={showLoginPw ? "Hide password" : "Show password"}
                className="absolute right-2.5 top-1/2 -translate-y-1/2 rounded p-1 text-fg-muted hover:text-fg cursor-pointer"
              >
                {showLoginPw ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
              </button>
            </div>
            {loginError && (
              <div className="rounded-lg border border-danger/40 bg-danger/10 px-3 py-2 text-xs text-danger">
                {loginError}
              </div>
            )}
            <Button
              variant="primary"
              className="w-full"
              disabled={loggingIn || !loginPassword.trim()}
              onClick={() => void handleDashboardLogin()}
            >
              <Lock className="h-3.5 w-3.5" />
              {loggingIn ? "Signing in..." : "Unlock Dashboard"}
            </Button>
          </div>
          <div className="flex items-center justify-between border-t border-border pt-3 text-[10px] text-fg-subtle">
            <span>Signed HttpOnly Cookie · 7d</span>
            <span>Brute-force protected</span>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-screen overflow-hidden bg-bg text-fg">
      <Sidebar
        activeTab={activeTab}
        onSelect={setActiveTab}
        collapsed={sidebarCollapsed}
        onToggleCollapse={() => setSidebarCollapsed((v) => !v)}
        mobileOpen={mobileOpen}
        onCloseMobile={() => setMobileOpen(false)}
        health={health}
        oauthCount={oauthProviders.length}
      />

      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <Header
          activeTab={activeTab}
          theme={theme}
          appearance={appearance}
          hasApiKey={hasApiKey}
          keysCount={keys.length}
          dashboardLocked={authState.locked}
          refreshing={refreshing}
          onOpenMobile={() => setMobileOpen(true)}
          onOpenPalette={() => setPaletteOpen(true)}
          onToggleTheme={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
          onToggleAppearance={() =>
            setAppearance((a) => (a === "editions" ? "classic" : "editions"))
          }
          onRefresh={() => void refreshAll()}
          onOpenApiKey={() => setApiKeyOpen(true)}
          onOpenSecurity={() => setActiveTab("security")}
          onLogout={() => void handleDashboardLogout()}
        />

        <main className="flex-1 overflow-y-auto px-4 py-6 lg:px-8">
          <div className="mx-auto max-w-7xl">
            {activeTab === "overview" && (
              <OverviewPage
                health={health}
                gamification={gamification}
                credentials={credentials}
                keys={keys}
                isDark={isDark}
                onNavigate={setActiveTab}
                onLaunchOAuth={launchOAuth}
                onOpenAddCred={(p) => setCredModal({ open: true, provider: p || "" })}
                onOpenApiKeys={() => setApiKeyOpen(true)}
                onRefresh={() => void refreshAll()}
                onToast={addToast}
              />
            )}
            {activeTab === "providers" && (
              <ProvidersPage
                credentials={credentials}
                oauthProviders={oauthProviders}
                isDark={isDark}
                onLaunchOAuth={launchOAuth}
                onOpenAddCred={(p) => setCredModal({ open: true, provider: p || "" })}
                onTryInPlayground={(provider, model) => {
                  setPlaygroundTarget({ provider, model });
                  setActiveTab("playground");
                }}
                onRefresh={() => void refreshAll()}
                onToast={addToast}
              />
            )}
            {activeTab === "playground" && (
              <PlaygroundPage
                isDark={isDark}
                credentials={credentials}
                combos={combos}
                initialSelection={playgroundTarget}
                onClearInitialSelection={() => setPlaygroundTarget(null)}
                onAfterChat={() => void refreshAll()}
                onToast={addToast}
              />
            )}
            {activeTab === "combos" && (
              <CombosPage
                combos={combos}
                strategies={strategies}
                isDark={isDark}
                onRefresh={() => void refreshAll()}
                onToast={addToast}
              />
            )}
            {activeTab === "oauth" && (
              <OAuthPage
                oauthProviders={oauthProviders}
                oauthStatus={oauthStatus}
                credentials={credentials}
                isDark={isDark}
                onLaunchOAuth={launchOAuth}
                onRefresh={() => void refreshAll()}
                onToast={addToast}
              />
            )}
            {activeTab === "quota" && (
              <QuotaProxyPage isDark={isDark} onToast={addToast} />
            )}
            {activeTab === "analytics" && (
              <AnalyticsLogsPage isDark={isDark} onToast={addToast} />
            )}
            {activeTab === "cli" && (
              <AliasesCliPage isDark={isDark} onToast={addToast} />
            )}
            {activeTab === "security" && (
              <SecurityPage
                keys={keys}
                guardrails={guardrails}
                dashboardLocked={authState.locked}
                onAuthChanged={() => void checkAuth()}
                onRefresh={() => void refreshAll()}
                onToast={addToast}
              />
            )}
            {activeTab === "optimization" && (
              <OptimizationPage
                health={health}
                onRefresh={() => void refreshAll()}
                onToast={addToast}
              />
            )}
            {activeTab === "mitm" && (
              <MitmPage
                mitm={mitm}
                isDark={isDark}
                onRefresh={() => void refreshAll()}
                onToast={addToast}
              />
            )}
            {activeTab === "a2a" && (
              <A2APage
                tasks={a2aTasks}
                onRefresh={() => void refreshAll()}
                onToast={addToast}
              />
            )}
            {activeTab === "memory" && (
              <MemorySkillsPage
                memories={memories}
                skills={skills}
                onRefresh={() => void refreshAll()}
                onToast={addToast}
              />
            )}
            {activeTab === "database" && (
              <DatabasePage
                db={db}
                onRefresh={() => void refreshAll()}
                onToast={addToast}
              />
            )}
          </div>
        </main>
      </div>

      {/* Global Overlays */}
      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        onSelectTab={setActiveTab}
        oauthProviders={oauthProviders}
        onLaunchOAuth={launchOAuth}
        isDark={isDark}
      />

      <OAuthModal
        request={oauthReq}
        isDark={isDark}
        onClose={() => setOAuthReq(null)}
        onSuccess={() => void refreshAll()}
        onToast={addToast}
      />

      <CredentialModal
        open={credModal.open}
        initialProvider={credModal.provider}
        isDark={isDark}
        onClose={() => setCredModal({ open: false, provider: "" })}
        onSaved={() => void refreshAll()}
        onToast={addToast}
      />

      <ApiKeyModal
        open={apiKeyOpen}
        keys={keys}
        onClose={() => setApiKeyOpen(false)}
        onChange={() => {
          setHasApiKey(!!getStoredApiKey());
          void refreshAll();
        }}
        onToast={addToast}
      />

      <ToastStack
        toasts={toasts}
        onDismiss={(id) => setToasts((prev) => prev.filter((t) => t.id !== id))}
      />
    </div>
  );
}
