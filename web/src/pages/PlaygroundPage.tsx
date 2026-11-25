import { useEffect, useMemo, useRef, useState } from "react";
import {
  Send,
  Trash2,
  Sparkles,
  Bot,
  User,
  Copy,
  SlidersHorizontal,
  Loader2,
  ChevronDown,
  Search,
  Check,
  GitBranch,
  Brain,
  Eye,
  RefreshCw,
  ExternalLink,
} from "lucide-react";
import {
  Card,
  Badge,
  Button,
  Input,
  PageHeader,
} from "@/components/ui/primitives";
import { ProviderIcon } from "@/components/ProviderIcon";
import {
  api,
  apiPost,
  type ComboItem,
  type CredentialItem,
  type ProviderSummary,
} from "@/lib/api";
import { cn, copyToClipboard } from "@/lib/utils";

interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  content: string;
  meta?: string[];
  isError?: boolean;
}

interface ModelOption {
  id: string; // full route id e.g. "antigravity/claude-sonnet-4-6" or combo name
  rawId: string; // model part e.g. "claude-sonnet-4-6"
  name: string;
  contextLength?: number | null;
  reasoning?: boolean;
  vision?: boolean;
  free?: boolean;
  synced?: boolean;
}

const COMBO_PROVIDER_ID = "__combos__";

function fmtCtx(n?: number | null): string | null {
  if (!n || n <= 0) return null;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n % 1_000_000 === 0 ? 0 : 1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`;
  return String(n);
}

function extractVerifyUrl(text: string): string | null {
  const m = text.match(/https:\/\/accounts\.google\.com\/[^\s"'<>)]+/i);
  if (m) return m[0];
  const m2 = text.match(/"validation_url"\s*:\s*"(https:\/\/[^"]+)"/i);
  if (m2) return m2[1];
  return null;
}

export function PlaygroundPage({
  isDark,
  credentials = [],
  combos = [],
  initialSelection,
  onClearInitialSelection,
  onAfterChat,
  onToast,
}: {
  isDark: boolean;
  credentials?: CredentialItem[];
  combos?: ComboItem[];
  initialSelection?: { provider: string; model: string } | null;
  onClearInitialSelection?: () => void;
  onAfterChat: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [aliases, setAliases] = useState<Array<{ alias: string; target: string }>>([]);

  // Two-level selection state: Provider -> Model
  const [selectedProvider, setSelectedProvider] = useState<string>("pollinations");
  const [selectedModelRaw, setSelectedModelRaw] = useState<string>("openai-fast");
  const [didInitDefault, setDidInitDefault] = useState(false);

  // Per-provider loaded models
  const [providerModels, setProviderModels] = useState<ModelOption[]>([]);
  const [loadingModels, setLoadingModels] = useState(false);
  const [providerPassthrough, setProviderPassthrough] = useState(false);

  // Dropdown popover open + search states
  const [provOpen, setProvOpen] = useState(false);
  const [provSearch, setProvSearch] = useState("");
  const [modelOpen, setModelOpen] = useState(false);
  const [modelSearch, setModelSearch] = useState("");
  const provDropdownRef = useRef<HTMLDivElement>(null);
  const modelDropdownRef = useRef<HTMLDivElement>(null);

  // Chat state
  const [temperature, setTemperature] = useState("0.7");
  const [systemPrompt, setSystemPrompt] = useState("");
  const [showSettings, setShowSettings] = useState(false);
  const [prompt, setPrompt] = useState("");
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [sending, setSending] = useState(false);
  const feedRef = useRef<HTMLDivElement>(null);

  // Load full provider directory + aliases once
  useEffect(() => {
    let alive = true;
    Promise.all([
      api<{ providers: ProviderSummary[] }>("/api/providers?limit=500"),
      api<{ aliases: Array<{ alias: string; target: string }> }>("/api/aliases"),
    ]).then(([pRes, aRes]) => {
      if (!alive) return;
      if (pRes.ok && Array.isArray(pRes.data?.providers)) {
        setProviders(pRes.data.providers);
      }
      if (aRes.ok && Array.isArray(aRes.data?.aliases)) {
        setAliases(aRes.data.aliases);
      }
    });
    return () => {
      alive = false;
    };
  }, []);

  // Apply explicit initialSelection (e.g. from Providers -> "Try in Playground")
  useEffect(() => {
    if (initialSelection?.provider) {
      setSelectedProvider(initialSelection.provider);
      setSelectedModelRaw(initialSelection.model || "");
      setDidInitDefault(true);
      onClearInitialSelection?.();
    }
  }, [initialSelection, onClearInitialSelection]);

  // Default to first connected credential once loaded (if user hasn't picked anything yet)
  useEffect(() => {
    if (!didInitDefault && !initialSelection && credentials.length > 0) {
      const first = credentials.find((c) => c.enabled) || credentials[0];
      if (first?.provider) {
        setSelectedProvider(first.provider);
        setSelectedModelRaw("");
        setDidInitDefault(true);
      }
    }
  }, [credentials, didInitDefault, initialSelection]);

  // Close popovers on outside click / Escape
  useEffect(() => {
    const onDocClick = (e: MouseEvent) => {
      if (provDropdownRef.current && !provDropdownRef.current.contains(e.target as Node)) {
        setProvOpen(false);
      }
      if (modelDropdownRef.current && !modelDropdownRef.current.contains(e.target as Node)) {
        setModelOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setProvOpen(false);
        setModelOpen(false);
      }
    };
    document.addEventListener("mousedown", onDocClick);
    window.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  // Fetch models whenever selectedProvider changes
  useEffect(() => {
    let alive = true;
    if (selectedProvider === COMBO_PROVIDER_ID) {
      const opts: ModelOption[] = [
        ...combos.map((c) => ({
          id: c.name,
          rawId: c.name,
          name: `Combo (${c.strategy}) · ${c.models?.length || 0} steps`,
        })),
        ...aliases.map((a) => ({
          id: a.alias,
          rawId: a.alias,
          name: `Alias → ${a.target}`,
        })),
      ];
      setProviderModels(opts);
      setProviderPassthrough(false);
      if (opts.length > 0 && !opts.some((o) => o.rawId === selectedModelRaw)) {
        setSelectedModelRaw(opts[0].rawId);
      }
      return;
    }

    setLoadingModels(true);
    Promise.all([
      api<any>(`/v1/models?provider=${encodeURIComponent(selectedProvider)}&limit=500`),
      api<any>(`/api/providers/${encodeURIComponent(selectedProvider)}`),
    ]).then(([mRes, pRes]) => {
      if (!alive) return;
      setLoadingModels(false);

      const byRaw = new Map<string, ModelOption>();

      // 1. Live synced models stored on connected credentials for this provider
      for (const c of credentials.filter((x) => x.provider === selectedProvider)) {
        const list: string[] = Array.isArray((c as any).models) ? (c as any).models : [];
        for (const raw of list) {
          if (!raw || raw === "*") continue;
          const cleanRaw = raw.startsWith(`${selectedProvider}/`)
            ? raw.slice(selectedProvider.length + 1)
            : raw;
          byRaw.set(cleanRaw, {
            id: `${selectedProvider}/${cleanRaw}`,
            rawId: cleanRaw,
            name: cleanRaw,
            synced: true,
          });
        }
      }

      // 2. Catalog specs from /v1/models?provider=...
      if (mRes.ok && Array.isArray(mRes.data?.data)) {
        for (const item of mRes.data.data) {
          const full = String(item.id || "");
          if (!full) continue;
          const raw = full.startsWith(`${selectedProvider}/`)
            ? full.slice(selectedProvider.length + 1)
            : full;
          const prev = byRaw.get(raw);
          byRaw.set(raw, {
            id: full,
            rawId: raw,
            name: item.name || raw,
            contextLength: item.context_length,
            reasoning: !!item.supports_reasoning,
            vision: !!item.supports_vision,
            free: !!item.free_tier,
            synced: prev?.synced || false,
          });
        }
      }

      // 3. Fallback simple model IDs from /api/providers/{id}
      if (pRes.ok && pRes.data) {
        setProviderPassthrough(!!pRes.data.passthrough_models);
        const rawList: string[] = Array.isArray(pRes.data.models) ? pRes.data.models : [];
        for (const raw of rawList) {
          if (!raw || raw === "*") continue;
          if (!byRaw.has(raw)) {
            byRaw.set(raw, {
              id: `${selectedProvider}/${raw}`,
              rawId: raw,
              name: raw,
            });
          }
        }
      } else {
        setProviderPassthrough(false);
      }

      const sorted = Array.from(byRaw.values()).sort((a, b) => {
        if (a.synced !== b.synced) return a.synced ? -1 : 1;
        return a.rawId.localeCompare(b.rawId);
      });
      setProviderModels(sorted);

      // Auto-select first model if current selection isn't valid for this provider
      setSelectedModelRaw((cur) => {
        if (cur && sorted.some((m) => m.rawId === cur)) return cur;
        if (sorted.length > 0) return sorted[0].rawId;
        return cur;
      });
    });

    return () => {
      alive = false;
    };
  }, [selectedProvider, credentials, combos, aliases]);

  // Connected provider IDs map
  const connectedCounts = useMemo(() => {
    const m: Record<string, number> = {};
    for (const c of credentials) {
      if (c.enabled) m[c.provider] = (m[c.provider] || 0) + 1;
    }
    return m;
  }, [credentials]);

  // Sorted & filtered provider groups for Dropdown 1
  const providerGroups = useMemo(() => {
    const q = provSearch.trim().toLowerCase();
    const matches = (p: ProviderSummary) =>
      !q ||
      p.id.toLowerCase().includes(q) ||
      (p.name || "").toLowerCase().includes(q) ||
      p.format.toLowerCase().includes(q);

    const allFiltered = providers.filter(matches);
    const connected = allFiltered.filter((p) => (connectedCounts[p.id] || 0) > 0);
    // Include any connected credential whose provider isn't in the first batch
    for (const pid of Object.keys(connectedCounts)) {
      if (!connected.some((p) => p.id === pid) && (!q || pid.toLowerCase().includes(q))) {
        connected.push({
          id: pid,
          name: pid,
          format: "openai",
          auth_type: "oauth",
          model_count: 0,
          is_free_tier: false,
          configured_accounts: connectedCounts[pid],
        });
      }
    }
    const free = allFiltered.filter((p) => p.is_free_tier && !(connectedCounts[p.id] > 0));
    const others = allFiltered
      .filter((p) => !p.is_free_tier && !(connectedCounts[p.id] > 0))
      .slice(0, 80);

    return { connected, free: free.slice(0, 40), others };
  }, [providers, provSearch, connectedCounts]);

  // Filtered model list for Dropdown 2
  const filteredModels = useMemo(() => {
    const q = modelSearch.trim().toLowerCase();
    if (!q) return providerModels;
    return providerModels.filter(
      (m) =>
        m.rawId.toLowerCase().includes(q) ||
        m.name.toLowerCase().includes(q) ||
        m.id.toLowerCase().includes(q)
    );
  }, [providerModels, modelSearch]);

  // Resolved full model string sent to /v1/chat/completions
  const fullModelId =
    selectedProvider === COMBO_PROVIDER_ID
      ? selectedModelRaw
      : selectedModelRaw
        ? `${selectedProvider}/${selectedModelRaw}`
        : selectedProvider;

  const selectedProviderMeta = providers.find((p) => p.id === selectedProvider);
  const selectedModelMeta = providerModels.find((m) => m.rawId === selectedModelRaw);

  const scrollToBottom = () => {
    window.setTimeout(() => {
      if (feedRef.current) feedRef.current.scrollTop = feedRef.current.scrollHeight;
    }, 30);
  };

  const sendMessage = async () => {
    const text = prompt.trim();
    if (!text || !fullModelId.trim() || sending) return;
    setPrompt("");
    const userMsg: ChatMessage = {
      id: Date.now(),
      role: "user",
      content: text,
      meta: [fullModelId],
    };
    const nextHistory = [...messages, userMsg];
    setMessages(nextHistory);
    scrollToBottom();

    const payloadMessages = [
      ...(systemPrompt.trim() ? [{ role: "system", content: systemPrompt.trim() }] : []),
      ...nextHistory.map((m) => ({ role: m.role, content: m.content })),
    ];

    setSending(true);
    const t0 = performance.now();
    const res = await apiPost<any>("/v1/chat/completions", {
      model: fullModelId.trim(),
      temperature: Number.parseFloat(temperature) || 0.7,
      messages: payloadMessages,
    });
    const ms = Math.round(performance.now() - t0);
    setSending(false);

    if (res.ok) {
      const content =
        res.data?.choices?.[0]?.message?.content ||
        (typeof res.data === "string" ? res.data : JSON.stringify(res.data, null, 2));
      const tags = [`${ms} ms`];
      const via = res.headers.get("x-omniroute-provider");
      const cache = res.headers.get("x-omniroute-cache");
      const saved = res.headers.get("x-omniroute-compression-saved");
      if (via) tags.push(`via ${via}`);
      if (cache) tags.push(`cache: ${cache}`);
      if (saved) tags.push(`-${saved} tok`);
      setMessages((prev) => [
        ...prev,
        { id: Date.now() + 1, role: "assistant", content, meta: tags },
      ]);
      onAfterChat();
    } else {
      setMessages((prev) => [
        ...prev,
        {
          id: Date.now() + 1,
          role: "assistant",
          content: `Error ${res.status}: ${res.error || "Request failed"}`,
          meta: [`${ms} ms`, "error"],
          isError: true,
        },
      ]);
    }
    scrollToBottom();
  };

  return (
    <div className="flex h-[calc(100vh-7.5rem)] flex-col">
      <PageHeader
        title="Live Chat Playground"
        description="Pilih provider & model langsung dari dropdown untuk menguji routing, OAuth, free-tier, cache & kompresi."
        actions={
          <>
            <Button
              variant={showSettings ? "primary" : "default"}
              size="sm"
              onClick={() => setShowSettings((v) => !v)}
            >
              <SlidersHorizontal className="h-3.5 w-3.5" />
              Parameters
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                setMessages([]);
                onToast("Conversation cleared", "info");
              }}
            >
              <Trash2 className="h-3.5 w-3.5" />
              Clear
            </Button>
          </>
        }
      />

      {/* Two-Level Cascading Selector Bar: Provider Dropdown -> Model Sub-Dropdown */}
      <Card className="mb-4 p-3.5">
        <div className="grid grid-cols-1 gap-3 md:grid-cols-12">
          {/* 1. Provider Dropdown (5 cols) */}
          <div ref={provDropdownRef} className="relative md:col-span-5">
            <label className="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-fg-muted">
              1. Provider ({providers.length || 394})
            </label>
            <button
              type="button"
              onClick={() => {
                setProvOpen((v) => !v);
                setModelOpen(false);
              }}
              className="flex h-10 w-full items-center justify-between gap-2 rounded-xl border border-border-strong bg-surface px-3 text-left text-sm transition-colors hover:border-accent/60 focus:border-ring focus:outline-none cursor-pointer"
            >
              <div className="flex items-center gap-2.5 min-w-0">
                {selectedProvider === COMBO_PROVIDER_ID ? (
                  <GitBranch className="h-4 w-4 shrink-0 text-accent-ink" />
                ) : (
                  <ProviderIcon providerId={selectedProvider} size={20} isDark={isDark} />
                )}
                <div className="min-w-0">
                  <div className="truncate text-xs font-semibold text-fg">
                    {selectedProvider === COMBO_PROVIDER_ID
                      ? "Combos & Aliases"
                      : selectedProviderMeta?.name || selectedProvider}
                  </div>
                  <div className="truncate font-mono text-[10px] text-fg-muted">
                    {selectedProvider === COMBO_PROVIDER_ID
                      ? `${combos.length} combos · ${aliases.length} aliases`
                      : `${selectedProvider}${connectedCounts[selectedProvider] ? ` · ${connectedCounts[selectedProvider]} connected` : ""}`}
                  </div>
                </div>
              </div>
              <ChevronDown
                className={cn(
                  "h-4 w-4 shrink-0 text-fg-muted transition-transform",
                  provOpen && "rotate-180"
                )}
              />
            </button>

            {provOpen && (
              <div className="absolute left-0 right-0 top-[calc(100%+6px)] z-40 overflow-hidden rounded-xl border border-border-strong bg-surface shadow-2xl">
                <div className="border-b border-border p-2">
                  <div className="relative">
                    <Search className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-fg-muted" />
                    <input
                      autoFocus
                      value={provSearch}
                      onChange={(e) => setProvSearch(e.target.value)}
                      placeholder="Cari provider (antigravity, claude, codex, pollinations...)"
                      className="h-8 w-full rounded-lg border border-border bg-surface-subtle pl-8 pr-2.5 text-xs text-fg placeholder:text-fg-muted/70 focus:border-ring focus:outline-none"
                    />
                  </div>
                </div>
                <div className="max-h-72 overflow-y-auto p-1.5 space-y-2">
                  {/* Combos & Aliases option */}
                  {(combos.length > 0 || aliases.length > 0) && (
                    <div>
                      <div className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-fg-muted">
                        Routing Combos &amp; Aliases
                      </div>
                      <button
                        type="button"
                        onClick={() => {
                          setSelectedProvider(COMBO_PROVIDER_ID);
                          setSelectedModelRaw("");
                          setProvOpen(false);
                          setProvSearch("");
                          setModelOpen(true);
                        }}
                        className={cn(
                          "flex w-full items-center justify-between gap-2 rounded-lg px-2.5 py-2 text-left text-xs hover:bg-surface-subtle cursor-pointer",
                          selectedProvider === COMBO_PROVIDER_ID && "bg-accent-subtle text-accent-ink"
                        )}
                      >
                        <div className="flex items-center gap-2 min-w-0">
                          <GitBranch className="h-4 w-4 shrink-0 text-accent-ink" />
                          <span className="font-medium">Combos &amp; Model Aliases</span>
                        </div>
                        <Badge variant="accent">{combos.length + aliases.length}</Badge>
                      </button>
                    </div>
                  )}

                  {/* Connected Accounts */}
                  {providerGroups.connected.length > 0 && (
                    <div>
                      <div className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-success">
                        Connected Accounts ({providerGroups.connected.length})
                      </div>
                      {providerGroups.connected.map((p) => (
                        <button
                          key={p.id}
                          type="button"
                          onClick={() => {
                            setSelectedProvider(p.id);
                            setSelectedModelRaw("");
                            setProvOpen(false);
                            setProvSearch("");
                            setModelOpen(true);
                          }}
                          className={cn(
                            "flex w-full items-center justify-between gap-2 rounded-lg px-2.5 py-2 text-left text-xs hover:bg-surface-subtle cursor-pointer",
                            selectedProvider === p.id && "bg-accent-subtle"
                          )}
                        >
                          <div className="flex items-center gap-2.5 min-w-0">
                            <ProviderIcon providerId={p.id} size={18} isDark={isDark} />
                            <div className="min-w-0">
                              <div className="truncate font-semibold text-fg">{p.name || p.id}</div>
                              <div className="truncate font-mono text-[10px] text-fg-muted">{p.id}</div>
                            </div>
                          </div>
                          <div className="flex items-center gap-1 shrink-0">
                            <Badge variant="success">{connectedCounts[p.id]}</Badge>
                            {p.model_count > 0 && (
                              <Badge variant="outline" className="font-mono">{p.model_count}m</Badge>
                            )}
                          </div>
                        </button>
                      ))}
                    </div>
                  )}

                  {/* Free Tier Providers */}
                  {providerGroups.free.length > 0 && (
                    <div>
                      <div className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-fg-muted">
                        Free Tier Providers ({providerGroups.free.length})
                      </div>
                      {providerGroups.free.map((p) => (
                        <button
                          key={p.id}
                          type="button"
                          onClick={() => {
                            setSelectedProvider(p.id);
                            setSelectedModelRaw("");
                            setProvOpen(false);
                            setProvSearch("");
                            setModelOpen(true);
                          }}
                          className={cn(
                            "flex w-full items-center justify-between gap-2 rounded-lg px-2.5 py-2 text-left text-xs hover:bg-surface-subtle cursor-pointer",
                            selectedProvider === p.id && "bg-accent-subtle"
                          )}
                        >
                          <div className="flex items-center gap-2.5 min-w-0">
                            <ProviderIcon providerId={p.id} size={18} isDark={isDark} />
                            <div className="min-w-0">
                              <div className="truncate font-medium text-fg">{p.name || p.id}</div>
                              <div className="truncate font-mono text-[10px] text-fg-muted">{p.id}</div>
                            </div>
                          </div>
                          <div className="flex items-center gap-1 shrink-0">
                            <Badge variant="accent">free</Badge>
                            <Badge variant="outline" className="font-mono">{p.model_count}m</Badge>
                          </div>
                        </button>
                      ))}
                    </div>
                  )}

                  {/* All Other Providers */}
                  {providerGroups.others.length > 0 && (
                    <div>
                      <div className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-fg-muted">
                        All Providers
                      </div>
                      {providerGroups.others.map((p) => (
                        <button
                          key={p.id}
                          type="button"
                          onClick={() => {
                            setSelectedProvider(p.id);
                            setSelectedModelRaw("");
                            setProvOpen(false);
                            setProvSearch("");
                            setModelOpen(true);
                          }}
                          className={cn(
                            "flex w-full items-center justify-between gap-2 rounded-lg px-2.5 py-2 text-left text-xs hover:bg-surface-subtle cursor-pointer",
                            selectedProvider === p.id && "bg-accent-subtle"
                          )}
                        >
                          <div className="flex items-center gap-2.5 min-w-0">
                            <ProviderIcon providerId={p.id} size={18} isDark={isDark} />
                            <div className="min-w-0">
                              <div className="truncate font-medium text-fg">{p.name || p.id}</div>
                              <div className="truncate font-mono text-[10px] text-fg-muted">{p.id}</div>
                            </div>
                          </div>
                          <Badge variant="outline" className="font-mono shrink-0">{p.model_count}m</Badge>
                        </button>
                      ))}
                    </div>
                  )}

                  {!providerGroups.connected.length &&
                    !providerGroups.free.length &&
                    !providerGroups.others.length && (
                      <div className="py-6 text-center text-xs text-fg-muted">
                        Provider tidak ditemukan.
                      </div>
                    )}
                </div>
              </div>
            )}
          </div>

          {/* 2. Sub-Dropdown: Model Selector for Selected Provider (7 cols) */}
          <div ref={modelDropdownRef} className="relative md:col-span-7">
            <div className="mb-1 flex items-center justify-between">
              <label className="text-[10px] font-semibold uppercase tracking-wider text-fg-muted">
                2. Model ({loadingModels ? "loading..." : `${providerModels.length} tersedia`})
              </label>
              <button
                type="button"
                onClick={async () => {
                  const ok = await copyToClipboard(fullModelId);
                  onToast(ok ? `Copied "${fullModelId}"` : "Copy failed", ok ? "info" : "error");
                }}
                className="inline-flex items-center gap-1 font-mono text-[10px] text-fg-muted hover:text-accent-ink cursor-pointer"
                title="Copy full route model ID"
              >
                <span>{fullModelId}</span>
                <Copy className="h-2.5 w-2.5" />
              </button>
            </div>
            <button
              type="button"
              onClick={() => {
                setModelOpen((v) => !v);
                setProvOpen(false);
              }}
              className="flex h-10 w-full items-center justify-between gap-2 rounded-xl border border-border-strong bg-surface px-3 text-left text-sm transition-colors hover:border-accent/60 focus:border-ring focus:outline-none cursor-pointer"
            >
              <div className="flex items-center gap-2 min-w-0">
                {loadingModels ? (
                  <RefreshCw className="h-3.5 w-3.5 shrink-0 animate-spin text-fg-muted" />
                ) : (
                  <Sparkles className="h-3.5 w-3.5 shrink-0 text-accent-ink" />
                )}
                <div className="min-w-0">
                  <div className="truncate font-mono text-xs font-semibold text-fg">
                    {selectedModelRaw || (loadingModels ? "Memuat daftar model..." : "Pilih atau ketik model...")}
                  </div>
                  {selectedModelMeta && selectedModelMeta.name !== selectedModelMeta.rawId && (
                    <div className="truncate text-[10px] text-fg-muted">
                      {selectedModelMeta.name}
                    </div>
                  )}
                </div>
              </div>
              <div className="flex items-center gap-1.5 shrink-0">
                {selectedModelMeta?.synced && <Badge variant="success">live</Badge>}
                {fmtCtx(selectedModelMeta?.contextLength) && (
                  <Badge variant="outline" className="font-mono hidden sm:inline-flex">
                    {fmtCtx(selectedModelMeta?.contextLength)}
                  </Badge>
                )}
                <ChevronDown
                  className={cn(
                    "h-4 w-4 text-fg-muted transition-transform",
                    modelOpen && "rotate-180"
                  )}
                />
              </div>
            </button>

            {modelOpen && (
              <div className="absolute left-0 right-0 top-[calc(100%+6px)] z-40 overflow-hidden rounded-xl border border-border-strong bg-surface shadow-2xl">
                <div className="border-b border-border p-2">
                  <div className="relative">
                    <Search className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-fg-muted" />
                    <input
                      autoFocus
                      value={modelSearch}
                      onChange={(e) => setModelSearch(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" && modelSearch.trim()) {
                          if (filteredModels.length > 0) {
                            setSelectedModelRaw(filteredModels[0].rawId);
                          } else {
                            setSelectedModelRaw(modelSearch.trim());
                          }
                          setModelOpen(false);
                          setModelSearch("");
                        }
                      }}
                      placeholder={`Cari model di ${selectedProvider} (atau ketik custom model ID + Enter)...`}
                      className="h-8 w-full rounded-lg border border-border bg-surface-subtle pl-8 pr-2.5 font-mono text-xs text-fg placeholder:text-fg-muted/70 focus:border-ring focus:outline-none"
                    />
                  </div>
                </div>

                <div className="max-h-72 overflow-y-auto p-1.5 divide-y divide-border/40">
                  {loadingModels ? (
                    <div className="py-8 text-center text-xs text-fg-muted">
                      Memuat daftar model {selectedProvider}...
                    </div>
                  ) : filteredModels.length === 0 ? (
                    <div className="p-4 text-center space-y-2">
                      <div className="text-xs text-fg-muted">
                        {providerPassthrough
                          ? `Provider "${selectedProvider}" mendukung model passthrough / dinamis.`
                          : "Tidak ada model yang cocok."}
                      </div>
                      {modelSearch.trim() && (
                        <Button
                          variant="primary"
                          size="sm"
                          onClick={() => {
                            setSelectedModelRaw(modelSearch.trim());
                            setModelOpen(false);
                            setModelSearch("");
                          }}
                        >
                          Pakai custom model: {modelSearch.trim()}
                        </Button>
                      )}
                    </div>
                  ) : (
                    filteredModels.map((m) => {
                      const active = m.rawId === selectedModelRaw;
                      const ctx = fmtCtx(m.contextLength);
                      return (
                        <button
                          key={m.id}
                          type="button"
                          onClick={() => {
                            setSelectedModelRaw(m.rawId);
                            setModelOpen(false);
                            setModelSearch("");
                          }}
                          className={cn(
                            "flex w-full items-center justify-between gap-2 rounded-lg px-2.5 py-2 text-left text-xs hover:bg-surface-subtle cursor-pointer",
                            active && "bg-accent-subtle"
                          )}
                        >
                          <div className="min-w-0 flex-1">
                            <div className="flex flex-wrap items-center gap-1.5">
                              <span className="truncate font-mono font-semibold text-fg">
                                {m.rawId}
                              </span>
                              {m.synced && <Badge variant="success">live</Badge>}
                              {m.free && <Badge variant="accent">free</Badge>}
                              {ctx && (
                                <Badge variant="outline" className="font-mono">
                                  {ctx}
                                </Badge>
                              )}
                              {m.reasoning && (
                                <Badge variant="info">
                                  <Brain className="h-3 w-3" />
                                </Badge>
                              )}
                              {m.vision && (
                                <Badge variant="default">
                                  <Eye className="h-3 w-3" />
                                </Badge>
                              )}
                            </div>
                            {m.name && m.name !== m.rawId && (
                              <div className="truncate text-[10px] text-fg-muted mt-0.5">
                                {m.name}
                              </div>
                            )}
                          </div>
                          {active && <Check className="h-4 w-4 shrink-0 text-accent-ink" />}
                        </button>
                      );
                    })
                  )}
                </div>

                {/* Custom model fallback option whenever user typed something not exact match */}
                {modelSearch.trim() &&
                  !filteredModels.some((m) => m.rawId === modelSearch.trim()) &&
                  filteredModels.length > 0 && (
                    <div className="border-t border-border bg-surface-subtle/50 p-2">
                      <button
                        type="button"
                        onClick={() => {
                          setSelectedModelRaw(modelSearch.trim());
                          setModelOpen(false);
                          setModelSearch("");
                        }}
                        className="w-full rounded-lg border border-dashed border-border-strong px-2.5 py-1.5 text-left font-mono text-[11px] text-fg hover:border-accent cursor-pointer"
                      >
                        + Pakai custom model ID: <strong>{modelSearch.trim()}</strong>
                      </button>
                    </div>
                  )}
              </div>
            )}
          </div>
        </div>

        {showSettings && (
          <div className="mt-3 grid grid-cols-1 gap-3 border-t border-border pt-3 sm:grid-cols-4">
            <div className="space-y-1">
              <label className="text-xs font-medium text-fg">Temperature ({temperature})</label>
              <input
                type="range"
                min="0"
                max="2"
                step="0.1"
                value={temperature}
                onChange={(e) => setTemperature(e.target.value)}
                className="w-full accent-[var(--accent)]"
              />
            </div>
            <div className="space-y-1 sm:col-span-3">
              <label className="text-xs font-medium text-fg">System Prompt (optional)</label>
              <Input
                value={systemPrompt}
                onChange={(e) => setSystemPrompt(e.target.value)}
                placeholder="You are a concise senior Rust engineer..."
              />
            </div>
          </div>
        )}
      </Card>

      {/* Conversation Feed */}
      <Card className="flex min-h-0 flex-1 flex-col overflow-hidden">
        <div ref={feedRef} className="flex-1 space-y-4 overflow-y-auto p-5">
          {messages.length === 0 ? (
            <div className="flex h-full flex-col items-center justify-center text-center">
              <div className="mb-3 flex h-12 w-12 items-center justify-center rounded-2xl border border-border bg-surface-subtle text-accent-ink">
                {selectedProvider === COMBO_PROVIDER_ID ? (
                  <GitBranch className="h-6 w-6" />
                ) : (
                  <ProviderIcon providerId={selectedProvider} size={26} isDark={isDark} />
                )}
              </div>
              <div className="text-sm font-semibold text-fg">
                Ready to chat with <code className="font-mono text-accent-ink">{fullModelId}</code>
              </div>
              <p className="mt-1 max-w-md text-xs text-fg-muted">
                Pilih provider di dropdown kiri dan sub-modelnya di dropdown kanan, lalu kirim prompt untuk cek latency, cache &amp; kompresi.
              </p>
              <div className="mt-4 flex flex-wrap justify-center gap-2">
                {[
                  "Explain Rust ownership in 3 bullet points.",
                  "Write a minimal Axum health endpoint.",
                  "Compare Claude Sonnet vs Gemini Pro.",
                ].map((sample) => (
                  <button
                    key={sample}
                    type="button"
                    onClick={() => setPrompt(sample)}
                    className="rounded-lg border border-border bg-surface-subtle px-3 py-1.5 text-xs text-fg-muted hover:border-border-strong hover:text-fg cursor-pointer"
                  >
                    {sample}
                  </button>
                ))}
              </div>
            </div>
          ) : (
            messages.map((m) => (
              <div
                key={m.id}
                className={cn(
                  "flex gap-3",
                  m.role === "user" ? "justify-end" : "justify-start"
                )}
              >
                {m.role === "assistant" && (
                  <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-border bg-surface-subtle text-accent-ink">
                    <Bot className="h-4 w-4" />
                  </div>
                )}

                <div
                  className={cn(
                    "max-w-[80%] space-y-1.5",
                    m.role === "user" ? "items-end text-right" : "items-start"
                  )}
                >
                  <div
                    className={cn(
                      "rounded-2xl px-4 py-3 text-sm leading-relaxed whitespace-pre-wrap break-words text-left",
                      m.role === "user"
                        ? "bg-accent text-accent-fg font-medium"
                        : m.isError
                          ? "border border-danger/40 bg-danger-bg text-danger"
                          : "border border-border bg-surface-subtle text-fg"
                    )}
                  >
                    {m.content}
                    {m.isError && (() => {
                      const verifyUrl = extractVerifyUrl(m.content);
                      if (!verifyUrl) return null;
                      return (
                        <div className="mt-3 flex flex-wrap items-center gap-2 border-t border-danger/25 pt-2.5">
                          <a
                            href={verifyUrl}
                            target="_blank"
                            rel="noreferrer"
                            className="inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-accent-fg hover:bg-accent-hover transition-colors"
                          >
                            <ExternalLink className="h-3.5 w-3.5" />
                            Verify Google Account in Browser
                          </a>
                          <button
                            type="button"
                            onClick={async () => {
                              const ok = await copyToClipboard(verifyUrl);
                              onToast(
                                ok ? "Verification URL copied" : "Copy failed",
                                ok ? "info" : "error"
                              );
                            }}
                            className="inline-flex items-center gap-1.5 rounded-lg border border-border-strong bg-surface px-2.5 py-1.5 text-xs font-medium text-fg hover:bg-surface-subtle cursor-pointer"
                          >
                            <Copy className="h-3.5 w-3.5" />
                            Copy Verify URL
                          </button>
                        </div>
                      );
                    })()}
                  </div>
                  {m.meta && m.meta.length > 0 && (
                    <div
                      className={cn(
                        "flex flex-wrap items-center gap-1.5",
                        m.role === "user" && "justify-end"
                      )}
                    >
                      {m.meta.map((tag, idx) => (
                        <Badge
                          key={idx}
                          variant={m.isError ? "danger" : idx === 0 ? "default" : "accent"}
                          className="font-mono text-[10px]"
                        >
                          {tag}
                        </Badge>
                      ))}
                      {m.role === "assistant" && !m.isError && (
                        <button
                          type="button"
                          onClick={async () => {
                            const ok = await copyToClipboard(m.content);
                            onToast(ok ? "Response copied" : "Copy failed", ok ? "info" : "error");
                          }}
                          className="inline-flex items-center gap-1 text-[10px] text-fg-muted hover:text-fg cursor-pointer"
                        >
                          <Copy className="h-3 w-3" /> Copy
                        </button>
                      )}
                    </div>
                  )}
                </div>

                {m.role === "user" && (
                  <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-border bg-surface-muted text-fg">
                    <User className="h-4 w-4" />
                  </div>
                )}
              </div>
            ))
          )}
        </div>

        {/* Input Composer */}
        <div className="border-t border-border bg-surface-subtle/40 p-3">
          <div className="flex items-end gap-2">
            <textarea
              rows={2}
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void sendMessage();
                }
              }}
              placeholder="Send a prompt... (Enter to send, Shift+Enter for newline)"
              className="flex-1 resize-none rounded-xl border border-border-strong bg-surface px-3.5 py-2.5 text-sm text-fg placeholder:text-fg-muted/70 focus:border-ring focus:outline-none focus:ring-2 focus:ring-ring/20"
            />
            <Button
              variant="primary"
              size="lg"
              disabled={sending || !prompt.trim()}
              onClick={() => void sendMessage()}
            >
              {sending ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}
              Send
            </Button>
          </div>
        </div>
      </Card>
    </div>
  );
}
