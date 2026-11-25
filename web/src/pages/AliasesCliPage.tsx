import { useCallback, useEffect, useState } from "react";
import {
  Terminal,
  ArrowRightLeft,
  Copy,
  Plus,
  Trash2,
  ExternalLink,
  FileCode2,
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
  PageHeader,
  EmptyState,
} from "@/components/ui/primitives";
import { ProviderIcon } from "@/components/ProviderIcon";
import { api, apiDelete, apiPost, setStoredApiKey } from "@/lib/api";
import { copyToClipboard } from "@/lib/utils";

const ALIAS_PRESETS = [
  { alias: "gpt-4o", target: "antigravity/claude-sonnet-4-6" },
  { alias: "claude-sonnet", target: "antigravity/claude-sonnet-4-6" },
  { alias: "fast", target: "antigravity/gemini-2.5-flash" },
];

export function AliasesCliPage({
  isDark,
  onToast,
}: {
  isDark: boolean;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [aliases, setAliases] = useState<Array<{ alias: string; target: string }>>([]);
  const [aliasInput, setAliasInput] = useState("");
  const [targetInput, setTargetInput] = useState("");
  const [cliData, setCliData] = useState<any>(null);
  const [filesCount, setFilesCount] = useState(0);
  const [batchesCount, setBatchesCount] = useState(0);

  const loadAll = useCallback(async () => {
    const [aRes, cRes, fRes, bRes] = await Promise.all([
      api<any>("/api/aliases"),
      api<any>("/api/cli-tools"),
      api<any>("/v1/files"),
      api<any>("/v1/batches"),
    ]);
    if (aRes.ok) setAliases(aRes.data?.aliases || []);
    if (cRes.ok) setCliData(cRes.data);
    if (fRes.ok) setFilesCount((fRes.data?.data || []).length);
    if (bRes.ok) setBatchesCount((bRes.data?.data || []).length);
  }, []);

  useEffect(() => {
    void loadAll();
  }, [loadAll]);

  const saveAlias = async (a?: string, t?: string) => {
    const alias = (a ?? aliasInput).trim();
    const target = (t ?? targetInput).trim();
    if (!alias || !target) {
      onToast("Both alias name and target model are required", "error");
      return;
    }
    const r = await apiPost("/api/aliases", { alias, target });
    if (r.ok) {
      setAliasInput("");
      setTargetInput("");
      onToast(`Alias "${alias}" -> "${target}" saved`, "success");
      void loadAll();
    } else {
      onToast(r.error || "Failed to save alias", "error");
    }
  };

  const removeAlias = async (alias: string) => {
    const r = await apiDelete(`/api/aliases/${encodeURIComponent(alias)}`);
    if (r.ok) {
      onToast(`Deleted alias "${alias}"`, "info");
      void loadAll();
    }
  };

  const copyText = async (text: string, label: string) => {
    const ok = await copyToClipboard(text);
    if (ok) {
      onToast(`${label} copied to clipboard`, "info");
    } else {
      onToast("Copy failed — select text manually", "error");
    }
  };

  const quickGenerateKey = async () => {
    const r = await apiPost("/api/keys", { label: "cli-tools" });
    if (r.ok && r.data?.key) {
      const k = String(r.data.key);
      setStoredApiKey(k);
      const copied = await copyToClipboard(k);
      onToast(
        copied ? "API key created, embedded in all snippets & copied!" : "API key created & embedded!",
        "success"
      );
      void loadAll();
    } else {
      onToast(r.error || "Failed to generate API key", "error");
    }
  };

  const tools: any[] = cliData?.tools || [];

  return (
    <div className="space-y-6">
      <PageHeader
        title="Model Aliases, CLI Tools & OpenAPI"
        description="Map client model names to any provider/combo and copy drop-in configs for Claude Code, Codex CLI, Cline, Continue, Aider, OpenCode & Cursor."
        actions={
          <div className="flex items-center gap-2">
            <Button variant="primary" size="sm" onClick={() => void quickGenerateKey()}>
              <Plus className="h-3.5 w-3.5" />
              1-Click API Key
            </Button>
            <a
              href="/api/openapi"
              target="_blank"
              rel="noreferrer"
              className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-border-strong bg-surface px-3 text-xs font-medium text-fg hover:bg-surface-subtle"
            >
              <FileCode2 className="h-3.5 w-3.5 text-accent-ink" />
              OpenAPI 3.1 JSON
              <ExternalLink className="h-3 w-3 text-fg-muted" />
            </a>
          </div>
        }
      />

      {/* Model Aliases */}
      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <ArrowRightLeft className="h-4 w-4 text-accent-ink" />
            <div>
              <CardTitle>Model Aliases</CardTitle>
              <CardDescription>
                Transparently rewrite incoming model IDs (e.g. <code className="font-mono">gpt-4o</code> -&gt; <code className="font-mono">antigravity/claude-sonnet-4-6</code>) before routing
              </CardDescription>
            </div>
          </div>
          <Badge variant="default">{aliases.length} active</Badge>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap gap-2">
            <Input
              value={aliasInput}
              onChange={(e) => setAliasInput(e.target.value)}
              placeholder="Alias (e.g. gpt-4o)"
              className="w-48 font-mono text-xs"
            />
            <Input
              value={targetInput}
              onChange={(e) => setTargetInput(e.target.value)}
              placeholder="Target (e.g. antigravity/claude-sonnet-4-6 or combo-name)"
              className="flex-1 min-w-[220px] font-mono text-xs"
            />
            <Button variant="primary" size="sm" onClick={() => void saveAlias()}>
              <Plus className="h-3.5 w-3.5" /> Add Alias
            </Button>
          </div>

          <div className="flex flex-wrap items-center gap-1.5 text-[11px] text-fg-muted">
            <span>Quick presets:</span>
            {ALIAS_PRESETS.map((p) => (
              <button
                key={p.alias}
                type="button"
                onClick={() => void saveAlias(p.alias, p.target)}
                className="rounded-md border border-border bg-surface-subtle px-2 py-0.5 font-mono text-[11px] text-fg hover:border-accent cursor-pointer"
              >
                {p.alias} → {p.target}
              </button>
            ))}
          </div>

          {aliases.length === 0 ? (
            <EmptyState
              title="No model aliases configured"
              description="Add an alias above to redirect any IDE/CLI default model name to your preferred provider or combo."
            />
          ) : (
            <div className="divide-y divide-border rounded-xl border border-border">
              {aliases.map((row) => {
                const targetProv = row.target.includes("/") ? row.target.split("/")[0] : "openai";
                return (
                  <div
                    key={row.alias}
                    className="flex items-center justify-between gap-3 px-3.5 py-2.5 text-xs"
                  >
                    <div className="flex items-center gap-2.5 min-w-0">
                      <ProviderIcon providerId={targetProv} size={18} isDark={isDark} />
                      <code className="rounded bg-accent-subtle px-2 py-0.5 font-mono font-semibold text-accent-ink">
                        {row.alias}
                      </code>
                      <span className="text-fg-muted">→</span>
                      <code className="truncate font-mono text-fg">{row.target}</code>
                    </div>
                    <Button variant="ghost" size="icon" onClick={() => void removeAlias(row.alias)}>
                      <Trash2 className="h-3.5 w-3.5 text-danger" />
                    </Button>
                  </div>
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>

      {/* CLI Tools Auto-Configurator */}
      <div className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <Terminal className="h-4 w-4 text-accent-ink" />
            <h2 className="text-sm font-semibold text-fg">CLI &amp; IDE Auto-Configurator</h2>
          </div>
          <div className="flex items-center gap-2 font-mono text-[11px] text-fg-muted">
            <span>Gateway: {cliData?.v1_url || "http://127.0.0.1:20128/v1"}</span>
            <span>·</span>
            <span>Files: {filesCount} · Batches: {batchesCount}</span>
          </div>
        </div>

        <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
          {tools.map((t) => (
            <Card key={t.id}>
              <CardHeader>
                <div className="flex items-center gap-2.5 min-w-0">
                  <ProviderIcon providerId={t.provider_icon || "openai"} size={22} isDark={isDark} />
                  <div className="min-w-0">
                    <CardTitle>{t.name}</CardTitle>
                    <CardDescription className="truncate font-mono text-[11px]">
                      {t.config_path}
                    </CardDescription>
                  </div>
                </div>
                <Button
                  variant="default"
                  size="sm"
                  onClick={() => void copyText(t.snippet, `${t.name} config`)}
                >
                  <Copy className="h-3.5 w-3.5" /> Copy
                </Button>
              </CardHeader>
              <CardContent className="space-y-2">
                <p className="text-xs text-fg-muted">{t.description}</p>
                <pre className="overflow-x-auto rounded-lg border border-border bg-bg p-3 font-mono text-[11px] leading-relaxed text-fg">
                  {t.snippet}
                </pre>
              </CardContent>
            </Card>
          ))}
        </div>
      </div>
    </div>
  );
}
