import { useEffect, useState } from "react";
import { KeyRound } from "lucide-react";
import { Dialog, Button, Input, Select } from "./ui/primitives";
import { ProviderIcon } from "./ProviderIcon";
import { apiPost, getStoredApiKey, setStoredApiKey } from "@/lib/api";

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
  onClose,
  onChange,
}: {
  open: boolean;
  onClose: () => void;
  onChange: () => void;
}) {
  const [val, setVal] = useState("");

  useEffect(() => {
    if (open) setVal(getStoredApiKey());
  }, [open]);

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Gateway API Key Authentication"
      description="Stored in browser localStorage and sent as Authorization: Bearer when strict auth is enabled."
      icon={<KeyRound className="h-5 w-5 text-accent-ink" />}
      maxWidth="max-w-md"
    >
      <div className="space-y-4">
        <div className="space-y-1.5">
          <label className="text-xs font-medium text-fg">Gateway API Key</label>
          <Input
            type="password"
            value={val}
            onChange={(e) => setVal(e.target.value)}
            placeholder="sk-omni-..."
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                setStoredApiKey(val);
                onChange();
                onClose();
              }
            }}
          />
        </div>
        <div className="flex justify-end gap-2">
          <Button
            variant="ghost"
            onClick={() => {
              setStoredApiKey("");
              onChange();
              onClose();
            }}
          >
            Clear Key
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              setStoredApiKey(val);
              onChange();
              onClose();
            }}
          >
            Save Key
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
