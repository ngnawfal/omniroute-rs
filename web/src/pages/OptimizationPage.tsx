import { Sparkles, Zap, Trash2 } from "lucide-react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Badge,
  Button,
  Switch,
  SegmentedControl,
  PageHeader,
} from "@/components/ui/primitives";
import { apiPost, apiPut, type GatewayHealth } from "@/lib/api";
import { fmtNumber } from "@/lib/utils";

export function OptimizationPage({
  health,
  onRefresh,
  onToast,
}: {
  health: GatewayHealth | null;
  onRefresh: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const cc = health?.compression?.config || {
    enabled: false,
    rtk_intensity: "standard",
    caveman_intensity: "off",
  };
  const cs = health?.compression?.stats || {
    requests_compressed: 0,
    total_tokens_saved: 0,
    total_compacted_turns: 0,
  };
  const sc = health?.cache?.config || {
    enabled: false,
    similarity_threshold: 0.85,
    require_zero_temperature: true,
    max_entries: 1000,
    ttl_ms: 1800000,
  };
  const ss = health?.cache?.stats || {
    entries: 0,
    direct_hits: 0,
    semantic_hits: 0,
    misses: 0,
    hit_rate: 0,
    tokens_saved: 0,
  };

  const updateCompression = async (patch: Partial<typeof cc>) => {
    const r = await apiPut("/api/settings/compression", { ...cc, ...patch });
    if (r.ok) {
      onToast("Compression config saved", "success");
      onRefresh();
    } else {
      onToast(r.error || "Failed to update compression", "error");
    }
  };

  const updateCache = async (patch: Partial<typeof sc>) => {
    const r = await apiPut("/api/settings/cache", { ...sc, ...patch });
    if (r.ok) {
      onToast("Semantic cache config saved", "success");
      onRefresh();
    } else {
      onToast(r.error || "Failed to update cache", "error");
    }
  };

  const clearCache = async () => {
    await apiPost("/api/cache/clear", {});
    onToast("Semantic cache cleared", "info");
    onRefresh();
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="Compression & Semantic Cache"
        description="Cut upstream token bills with error-preserving RTK + Caveman EN/ID compression and per-API-key dual-layer semantic caching."
      />

      <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
        {/* Compression Card */}
        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <Sparkles className="h-4 w-4 text-accent-ink" />
              <CardTitle>RTK + Caveman Compression</CardTitle>
            </div>
            <Badge variant="accent">{fmtNumber(cs.total_tokens_saved)} tokens saved</Badge>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="divide-y divide-border rounded-xl border border-border bg-surface-subtle/35 px-4">
              <div className="flex items-center justify-between gap-4 py-3">
                <div>
                  <div className="text-xs font-semibold text-fg">Enable Prompt &amp; Tool Compression</div>
                  <div className="text-[11px] text-fg-muted">Preserves stack traces &amp; errors while trimming log noise</div>
                </div>
                <Switch
                  checked={!!cc.enabled}
                  onChange={(v) => updateCompression({ enabled: v })}
                />
              </div>

              <div className="flex flex-wrap items-center justify-between gap-3 py-3">
                <div>
                  <div className="text-xs font-semibold text-fg">RTK Tool-Output Intensity</div>
                  <div className="text-[11px] text-fg-muted">Compresses verbose build/test logs</div>
                </div>
                <SegmentedControl
                  value={cc.rtk_intensity as "off" | "light" | "standard" | "aggressive"}
                  onChange={(v) => updateCompression({ rtk_intensity: v })}
                  options={[
                    { value: "off", label: "Off" },
                    { value: "light", label: "Light" },
                    { value: "standard", label: "Standard" },
                    { value: "aggressive", label: "Aggressive" },
                  ]}
                />
              </div>

              <div className="flex flex-wrap items-center justify-between gap-3 py-3">
                <div>
                  <div className="text-xs font-semibold text-fg">Caveman Intensity (EN / ID)</div>
                  <div className="text-[11px] text-fg-muted">Removes filler phrasing, preserves code blocks</div>
                </div>
                <SegmentedControl
                  value={cc.caveman_intensity as "off" | "lite" | "full" | "ultra"}
                  onChange={(v) => updateCompression({ caveman_intensity: v })}
                  options={[
                    { value: "off", label: "Off" },
                    { value: "lite", label: "Lite" },
                    { value: "full", label: "Full" },
                    { value: "ultra", label: "Ultra" },
                  ]}
                />
              </div>
            </div>

            <div className="grid grid-cols-2 gap-3">
              <div className="rounded-xl border border-border bg-surface-subtle/50 p-3.5">
                <div className="font-mono text-2xl font-semibold text-fg">
                  {fmtNumber(cs.requests_compressed)}
                </div>
                <div className="text-[11px] text-fg-muted">Requests compressed</div>
              </div>
              <div className="rounded-xl border border-border bg-surface-subtle/50 p-3.5">
                <div className="font-mono text-2xl font-semibold text-fg">
                  {fmtNumber(cs.total_compacted_turns)}
                </div>
                <div className="text-[11px] text-fg-muted">Compacted turns</div>
              </div>
            </div>
          </CardContent>
        </Card>

        {/* Semantic Cache Card */}
        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <Zap className="h-4 w-4 text-success" />
              <CardTitle>Dual-Layer Semantic Cache</CardTitle>
            </div>
            <Button variant="danger" size="sm" onClick={clearCache}>
              <Trash2 className="h-3.5 w-3.5" />
              Clear Cache
            </Button>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="divide-y divide-border rounded-xl border border-border bg-surface-subtle/35 px-4">
              <div className="flex items-center justify-between gap-4 py-3">
                <div>
                  <div className="text-xs font-semibold text-fg">Enable Semantic Cache</div>
                  <div className="text-[11px] text-fg-muted">Exact SHA-256 + sparse TF cosine match per API key</div>
                </div>
                <Switch
                  checked={!!sc.enabled}
                  onChange={(v) => updateCache({ enabled: v })}
                />
              </div>

              <div className="flex items-center justify-between gap-4 py-3">
                <div>
                  <div className="text-xs font-semibold text-fg">Require Temperature = 0</div>
                  <div className="text-[11px] text-fg-muted">Avoid caching stochastic high-temperature completions</div>
                </div>
                <Switch
                  checked={!!sc.require_zero_temperature}
                  onChange={(v) => updateCache({ require_zero_temperature: v })}
                />
              </div>

              <div className="flex items-center justify-between gap-4 py-3">
                <div>
                  <div className="text-xs font-semibold text-fg">
                    Cosine Similarity Threshold{" "}
                    <span className="font-mono text-accent-ink">
                      {Number(sc.similarity_threshold).toFixed(2)}
                    </span>
                  </div>
                  <div className="text-[11px] text-fg-muted">Higher = stricter semantic cache matching</div>
                </div>
                <input
                  type="range"
                  min="0.50"
                  max="0.99"
                  step="0.01"
                  value={sc.similarity_threshold}
                  onChange={(e) =>
                    updateCache({ similarity_threshold: Number.parseFloat(e.target.value) })
                  }
                  className="w-36 accent-[var(--accent)]"
                />
              </div>
            </div>

            <div className="grid grid-cols-3 gap-3">
              <div className="rounded-xl border border-border bg-surface-subtle/50 p-3.5">
                <div className="font-mono text-2xl font-semibold text-success">
                  {fmtNumber(ss.direct_hits)}
                </div>
                <div className="text-[11px] text-fg-muted">Exact hits</div>
              </div>
              <div className="rounded-xl border border-border bg-surface-subtle/50 p-3.5">
                <div className="font-mono text-2xl font-semibold text-accent-ink">
                  {fmtNumber(ss.semantic_hits)}
                </div>
                <div className="text-[11px] text-fg-muted">Semantic hits</div>
              </div>
              <div className="rounded-xl border border-border bg-surface-subtle/50 p-3.5">
                <div className="font-mono text-2xl font-semibold text-fg">
                  {fmtNumber(ss.misses)}
                </div>
                <div className="text-[11px] text-fg-muted">Cache misses</div>
              </div>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
