import { useCallback, useEffect, useRef, useState } from "react";
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
import {
  api,
  getStoredApiKey,
  type ComboItem,
  type CredentialItem,
  type GamificationStatus,
  type GatewayHealth,
  type OAuthProviderMeta,
} from "./lib/api";

export default function App() {
  const [activeTab, setActiveTab] = useState<NavTab>("overview");
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
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

  // Modals
  const [oauthReq, setOAuthReq] = useState<OAuthLaunchRequest | null>(null);
  const [credModal, setCredModal] = useState<{ open: boolean; provider: string }>({
    open: false,
    provider: "",
  });

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

  const refreshAll = useCallback(async () => {
    setRefreshing(true);
    setHasApiKey(!!getStoredApiKey());
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
    if (memRes.ok) setMemories(memRes.data?.memories || []);
    if (skRes.ok) setSkills(skRes.data?.skills || []);
    if (oaRes.ok) {
      setOAuthProviders(oaRes.data?.providers || []);
      setOAuthStatus(oaRes.data?.status);
    }
    if (keysRes.ok) setKeys(keysRes.data?.keys || []);
    if (credsRes.ok) setCredentials(credsRes.data?.credentials || []);
    if (combosRes.ok) {
      setCombos(combosRes.data?.combos || []);
      setStrategies(combosRes.data?.strategies || []);
    }
    if (tasksRes.ok) setA2ATasks(tasksRes.data?.tasks || []);
  }, []);

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
          refreshing={refreshing}
          onOpenMobile={() => setMobileOpen(true)}
          onOpenPalette={() => setPaletteOpen(true)}
          onToggleTheme={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
          onToggleAppearance={() =>
            setAppearance((a) => (a === "editions" ? "classic" : "editions"))
          }
          onRefresh={() => void refreshAll()}
          onOpenApiKey={() => setApiKeyOpen(true)}
        />

        <main className="flex-1 overflow-y-auto px-4 py-6 lg:px-8">
          <div className="mx-auto max-w-7xl">
            {activeTab === "overview" && (
              <OverviewPage
                health={health}
                gamification={gamification}
                credentials={credentials}
                isDark={isDark}
                onNavigate={setActiveTab}
                onLaunchOAuth={launchOAuth}
                onOpenAddCred={(p) => setCredModal({ open: true, provider: p || "" })}
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
                onRefresh={() => void refreshAll()}
                onToast={addToast}
              />
            )}
            {activeTab === "playground" && (
              <PlaygroundPage
                isDark={isDark}
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
            {activeTab === "security" && (
              <SecurityPage
                keys={keys}
                guardrails={guardrails}
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
        onClose={() => setApiKeyOpen(false)}
        onChange={() => {
          setHasApiKey(!!getStoredApiKey());
          addToast(getStoredApiKey() ? "Gateway API key saved" : "Gateway API key cleared", "info");
          void refreshAll();
        }}
      />

      <ToastStack
        toasts={toasts}
        onDismiss={(id) => setToasts((prev) => prev.filter((t) => t.id !== id))}
      />
    </div>
  );
}
