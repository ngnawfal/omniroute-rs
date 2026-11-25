import { useEffect, useState } from "react";
import { KeyRound, Copy, Plus, Trash2, Eye, EyeOff, Check, Sparkles } from "lucide-react";
import { Dialog, Button, Input, Select, Badge } from "./ui/primitives";
import { ProviderIcon } from "./ProviderIcon";
import { apiDelete, apiPost, getStoredApiKey, setStoredApiKey } from "@/lib/api";
import { copyToClipboard } from "@/lib/utils";

export function CredentialModal({
  open,
  initialProvider,
  isDark,
  onClose,
  onSaved,
  onToast,
}: {
  open: boolean;
  initialProvider: string;
  isDark: boolean;
  onClose: () => void;
  onSaved: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [provider, setProvider] = useState("");
  const [connId, setConnId] = useState("default");
  const [apiKey, setApiKey] = useState("");
  const [authType, setAuthType] = useState("bearer");
  const [label, setLabel] = useState("");
  const [customUrl, setCustomUrl] = useState("");
  const [proxyUrl, setProxyUrl] = useState("");
  const [psjText, setPsjText] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open) {
      setProvider(initialProvider || "");
      setConnId("default");
      setApiKey("");
      setAuthType(initialProvider === "anthropic" ? "x-api-key" : initialProvider === "azure" ? "api-key" : "bearer");
      setLabel("");
      setCustomUrl("");
      setProxyUrl("");
      setPsjText("");
    }
  }, [open, initialProvider]);

  const handleSave = async () => {
    if (!provider.trim()) {
      onToast("Provider ID is required", "error");
      return;
    }
    let psj: Record<string, any> = {};
    if (psjText.trim()) {
      try {
        psj = JSON.parse(psjText.trim());
      } catch {
        onToast("Invalid provider-specific JSON", "error");
        return;
      }
    }
    if (proxyUrl.trim()) {
      psj.proxy_url = proxyUrl.trim();
    }
    setSaving(true);
    const res = await apiPost("/api/providers/credentials", {
      provider: provider.trim(),
      connection_id: connId.trim() || "default",
      auth_type: authType,
      api_key: apiKey,
      base_urls: customUrl.trim() ? [customUrl.trim()] : [],
      models: [],
      label: label.trim(),
      provider_specific_json: psj,
    });
    setSaving(false);
    if (res.ok) {
      onToast(`Credential saved for ${provider.trim()}`, "success");
      onSaved();
      onClose();
    } else {
      onToast(res.error || "Failed to save credential", "error");
    }
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={provider ? `Connect ${provider}` : "Connect Provider Credential"}
      description="Store an API key or cloud credential in encrypted-ready SQLite WAL storage"
      icon={
        provider ? (
          <ProviderIcon providerId={provider} size={24} isDark={isDark} />
        ) : (
          <KeyRound className="h-5 w-5 text-accent-ink" />
        )
      }
    >
      <div className="space-y-3">
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
          <div className="space-y-1 sm:col-span-2">
            <label className="text-xs font-medium text-fg">Provider ID</label>
            <Input
              value={provider}
              onChange={(e) => setProvider(e.target.value)}
              placeholder="openai, anthropic, deepseek, groq, bedrock..."
            />
          </div>
          <div className="space-y-1">
            <label className="text-xs font-medium text-fg">Connection ID</label>
            <Input value={connId} onChange={(e) => setConnId(e.target.value)} placeholder="default" />
          </div>
        </div>

        <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
          <div className="space-y-1 sm:col-span-2">
            <label className="text-xs font-medium text-fg">API Key / Secret</label>
            <Input
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder="sk-..."
            />
          </div>
          <div className="space-y-1">
            <label className="text-xs font-medium text-fg">Auth Header</label>
            <Select value={authType} onChange={(e) => setAuthType(e.target.value)} className="w-full">
              <option value="bearer">Bearer</option>
              <option value="x-api-key">x-api-key</option>
              <option value="oauth">OAuth Bearer</option>
              <option value="api-key">api-key (Azure)</option>
              <option value="x-goog-api-key">x-goog-api-key</option>
            </Select>
          </div>
        </div>

        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <div className="space-y-1">
            <label className="text-xs font-medium text-fg">Account Label (optional)</label>
            <Input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Production Key" />
          </div>
          <div className="space-y-1">
            <label className="text-xs font-medium text-fg">Custom Base URL (optional)</label>
            <Input value={customUrl} onChange={(e) => setCustomUrl(e.target.value)} placeholder="https://..." />
          </div>
        </div>

        <div className="space-y-1">
          <label className="text-xs font-medium text-fg">
            Per-Account Upstream Proxy URL (optional — <code className="font-mono">http://</code>, <code className="font-mono">socks5h://</code>, or <code className="font-mono">direct</code>)
          </label>
          <Input
            value={proxyUrl}
            onChange={(e) => setProxyUrl(e.target.value)}
            placeholder="socks5h://user:pass@127.0.0.1:1080"
            className="font-mono text-xs"
          />
        </div>

        <div className="space-y-1">
          <label className="text-xs font-medium text-fg">
            Provider-Specific JSON (optional — Bedrock region/IAM, Vertex project_id, Azure api_version)
          </label>
          <Input
            value={psjText}
            onChange={(e) => setPsjText(e.target.value)}
            placeholder='{"region":"us-east-1"}'
            className="font-mono text-xs"
          />
        </div>

        <div className="flex justify-end gap-2 pt-2">
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button variant="primary" disabled={saving} onClick={handleSave}>
            {saving ? "Saving..." : "Save Credential"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

export function ApiKeyModal({
  open,
  keys = [],
  onClose,
  onChange,
  onToast,
}: {
  open: boolean;
  keys?: any[];
  onClose: () => void;
  onChange: () => void;
  onToast?: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [label, setLabel] = useState("");
  const [generating, setGenerating] = useState(false);
  const [freshKey, setFreshKey] = useState<string | null>(null);
  const [revealed, setRevealed] = useState<Record<string, boolean>>({});
  const [manualVal, setManualVal] = useState("");
  const [showManual, setShowManual] = useState(false);
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const origin = typeof window !== "undefined" ? window.location.origin : "http://127.0.0.1:20128";
  const activeKey = getStoredApiKey();

  useEffect(() => {
    if (open) {
      setManualVal(getStoredApiKey());
      setFreshKey(null);
      setLabel("");
      setShowManual(false);
    }
  }, [open]);

  const doCopy = async (text: string, id: string, msg: string) => {
    const ok = await copyToClipboard(text);
    if (ok) {
      setCopiedId(id);
      setTimeout(() => setCopiedId((prev) => (prev === id ? null : prev)), 1800);
      onToast?.(msg, "info");
    } else {
      onToast?.("Copy failed — select text manually", "error");
    }
  };

  const quickGenerate = async () => {
    setGenerating(true);
    const r = await apiPost("/api/keys", { label: label.trim() || "my-app" });
    setGenerating(false);
    if (r.ok && r.data?.key) {
      const k = String(r.data.key);
      // Auto-save to localStorage so dashboard never locks itself out after creating first key!
      setStoredApiKey(k);
      setManualVal(k);
      setFreshKey(k);
      setLabel("");
      const copied = await copyToClipboard(k);
      onToast?.(
        copied
          ? "API key created, activated & copied to clipboard!"
          : "API key created & activated!",
        "success"
      );
      onChange();
    } else {
      onToast?.(r.error || "Failed to generate API key", "error");
    }
  };

  const deleteKey = async (keyVal: string) => {
    const r = await apiDelete(`/api/keys/${encodeURIComponent(keyVal)}`);
    if (r.ok) {
      if (getStoredApiKey() === keyVal) {
        const next = keys.find((k) => k.key && k.key !== keyVal && !k.disabled);
        setStoredApiKey(next?.key || "");
      }
      onToast?.("API key deleted", "info");
      onChange();
    } else {
      onToast?.(r.error || "Delete failed", "error");
    }
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Connect & Gateway API Keys"
      description="1-click API key generator & endpoints for Claude Code, Cline, Cursor, Codex CLI & OpenAI SDK."
      icon={<KeyRound className="h-5 w-5 text-accent-ink" />}
      maxWidth="max-w-xl"
    >
      <div className="space-y-4">
        {/* Base URLs ready to copy */}
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
          {[
            { id: "v1", label: "OpenAI Base URL", url: `${origin}/v1` },
            { id: "anth", label: "Anthropic Base URL", url: origin },
          ].map((ep) => (
            <button
              key={ep.id}
              type="button"
              onClick={() => void doCopy(ep.url, ep.id, `${ep.label} copied`)}
              className="flex items-center justify-between gap-2 rounded-xl border border-border bg-surface-subtle/60 px-3 py-2 text-left hover:border-accent/60 cursor-pointer"
            >
              <div className="min-w-0">
                <div className="text-[10px] font-semibold uppercase tracking-wider text-fg-muted">
                  {ep.label}
                </div>
                <div className="truncate font-mono text-xs text-fg">{ep.url}</div>
              </div>
              {copiedId === ep.id ? (
                <Check className="h-3.5 w-3.5 shrink-0 text-success" />
              ) : (
                <Copy className="h-3.5 w-3.5 shrink-0 text-fg-muted" />
              )}
            </button>
          ))}
        </div>

        {/* 1-Click Generate Key */}
        <div className="rounded-xl border border-accent/35 bg-accent-subtle/40 p-3.5 space-y-2.5">
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-1.5 text-xs font-semibold text-fg">
              <Sparkles className="h-3.5 w-3.5 text-accent-ink" />
              <span>1-Click Generate &amp; Copy API Key</span>
            </div>
            <Badge variant={keys.length ? "accent" : "success"}>
              {keys.length ? `${keys.length} key(s) · Strict` : "Open Local Mode"}
            </Badge>
          </div>
          <div className="flex gap-2">
            <Input
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder="Label opsional (misal: cursor, cline, claude-code)"
              className="flex-1"
              onKeyDown={(e) => {
                if (e.key === "Enter") void quickGenerate();
              }}
            />
            <Button variant="primary" disabled={generating} onClick={() => void quickGenerate()}>
              <Plus className="h-3.5 w-3.5" />
              {generating ? "Creating..." : "Generate & Copy"}
            </Button>
          </div>
          {!keys.length && (
            <p className="text-[11px] text-fg-muted">
              Tips: Tanpa API key pun gateway sudah jalan di mode lokal — di tool lain cukup isi API key bebas (misal{" "}
              <button
                type="button"
                onClick={() => void doCopy("omniroute-local", "local-key", "Copied 'omniroute-local'")}
                className="font-mono underline text-fg cursor-pointer"
              >
                omniroute-local
              </button>
              ). Klik <strong>Generate &amp; Copy</strong> jika ingin kunci resmi.
            </p>
          )}
          {freshKey && (
            <div className="flex items-center justify-between gap-2 rounded-lg border border-accent/50 bg-bg px-3 py-2">
              <code className="truncate font-mono text-xs text-fg">{freshKey}</code>
              <Button
                variant="primary"
                size="sm"
                onClick={() => void doCopy(freshKey, "fresh", "API key copied")}
              >
                {copiedId === "fresh" ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                Copy
              </Button>
            </div>
          )}
        </div>

        {/* Existing Keys List with Show / Copy / Delete */}
        {keys.length > 0 && (
          <div className="space-y-1.5">
            <div className="text-xs font-semibold text-fg">Active Gateway API Keys</div>
            <div className="max-h-44 overflow-y-auto divide-y divide-border rounded-xl border border-border">
              {keys.map((k: any, idx: number) => {
                const fullKey: string = k.key || "";
                const id = k.key_hash || String(idx);
                const isShown = !!revealed[id];
                const isCurrent = !!fullKey && activeKey === fullKey;
                return (
                  <div
                    key={id}
                    className="flex items-center justify-between gap-2 px-3 py-2 text-xs"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-1.5">
                        <span className="font-medium text-fg truncate">{k.label || "API Key"}</span>
                        {isCurrent && <Badge variant="success">Active</Badge>}
                      </div>
                      <code className="block truncate font-mono text-[11px] text-fg-muted">
                        {isShown && fullKey ? fullKey : k.key_preview}
                      </code>
                    </div>
                    <div className="flex items-center gap-1 shrink-0">
                      {fullKey && (
                        <>
                          <Button
                            variant="ghost"
                            size="icon"
                            title={isShown ? "Hide key" : "Show full key"}
                            onClick={() =>
                              setRevealed((prev) => ({ ...prev, [id]: !prev[id] }))
                            }
                          >
                            {isShown ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
                          </Button>
                          <Button
                            variant="default"
                            size="sm"
                            onClick={() => {
                              setStoredApiKey(fullKey);
                              void doCopy(fullKey, id, "API key copied & set active");
                              onChange();
                            }}
                          >
                            {copiedId === id ? (
                              <Check className="h-3.5 w-3.5 text-success" />
                            ) : (
                              <Copy className="h-3.5 w-3.5" />
                            )}
                            Copy
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            title="Delete key"
                            onClick={() => void deleteKey(fullKey)}
                          >
                            <Trash2 className="h-3.5 w-3.5 text-danger" />
                          </Button>
                        </>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {/* Collapsible manual key paste (for second device when strict auth is already on) */}
        <div className="border-t border-border pt-2.5">
          <button
            type="button"
            onClick={() => setShowManual((v) => !v)}
            className="text-[11px] text-fg-muted hover:text-fg underline cursor-pointer"
          >
            {showManual ? "Hide manual key input" : "Opening from another device? Paste existing key here"}
          </button>
          {showManual && (
            <div className="mt-2 flex gap-2">
              <Input
                type="password"
                value={manualVal}
                onChange={(e) => setManualVal(e.target.value)}
                placeholder="Paste existing sk-omni-... key"
                className="flex-1 font-mono text-xs"
              />
              <Button
                variant="default"
                size="sm"
                onClick={() => {
                  setStoredApiKey(manualVal);
                  onChange();
                  onToast?.(manualVal.trim() ? "Key saved to browser" : "Key cleared", "info");
                }}
              >
                Use Key
              </Button>
            </div>
          )}
        </div>
      </div>
    </Dialog>
  );
}
