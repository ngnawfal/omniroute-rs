import { useState } from "react";
import {
  GitBranch,
  Plus,
  Trash2,
  Play,
  ArrowRight,
  X,
} from "lucide-react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Badge,
  Button,
  Input,
  Select,
  PageHeader,
  EmptyState,
} from "@/components/ui/primitives";
import { ProviderIcon } from "@/components/ProviderIcon";
import { apiDelete, apiPost, type ComboItem } from "@/lib/api";

interface StepDraft {
  provider: string;
  model: string;
  weight: string;
}

export function CombosPage({
  combos,
  strategies,
  isDark,
  onRefresh,
  onToast,
}: {
  combos: ComboItem[];
  strategies: string[];
  isDark: boolean;
  onRefresh: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [name, setName] = useState("");
  const [strategy, setStrategy] = useState(strategies[0] || "priority");
  const [steps, setSteps] = useState<StepDraft[]>([
    { provider: "antigravity", model: "claude-sonnet-4-6", weight: "1" },
    { provider: "groq", model: "openai/gpt-oss-120b", weight: "1" },
  ]);
  const [saving, setSaving] = useState(false);

  const [simModel, setSimModel] = useState("openai/gpt-6.1-sol");
  const [simStrat, setSimStrat] = useState("");
  const [simResult, setSimResult] = useState<any>(null);

  const saveCombo = async () => {
    if (!name.trim()) {
      onToast("Combo name is required", "error");
      return;
    }
    const models = steps
      .map((s) => ({
        provider: s.provider.trim(),
        model: s.model.trim(),
        weight: Number.parseFloat(s.weight) || 1.0,
      }))
      .filter((s) => s.provider && s.model);
    if (!models.length) {
      onToast("Add at least one target step", "error");
      return;
    }
    setSaving(true);
    const r = await apiPost("/api/combos", { name: name.trim(), strategy, models });
    setSaving(false);
    if (r.ok) {
      setName("");
      onToast("Combo saved", "success");
      onRefresh();
    } else {
      onToast(r.error || "Failed to save combo", "error");
    }
  };

  const delCombo = async (id: string) => {
    await apiDelete(`/api/combos/${encodeURIComponent(id)}`);
    onToast("Combo deleted", "info");
    onRefresh();
  };

  const runSimulate = async () => {
    const body: any = { model: simModel.trim() };
    if (simStrat) body.strategy = simStrat;
    const r = await apiPost("/api/combos/simulate", body);
    if (r.ok) setSimResult(r.data);
    else onToast(r.error || "Simulation failed", "error");
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="Combos & Smart Routing"
        description="Chain providers with automatic failover, round-robin, latency-aware, cost-optimized, or weighted routing across 22 strategies."
        badge={<Badge variant="accent">{strategies.length || 22} strategies</Badge>}
      />

      <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
        {/* Combo Builder */}
        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <GitBranch className="h-4 w-4 text-accent-ink" />
              <CardTitle>Create Routing Combo</CardTitle>
            </div>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div className="space-y-1">
                <label className="text-xs font-medium text-fg">Combo Name</label>
                <Input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="coding-fast-fallback"
                />
              </div>
              <div className="space-y-1">
                <label className="text-xs font-medium text-fg">Routing Strategy</label>
                <Select
                  value={strategy}
                  onChange={(e) => setStrategy(e.target.value)}
                  className="w-full"
                >
                  {(strategies.length ? strategies : ["priority", "round-robin", "weighted", "least-latency", "cost-optimized"]).map(
                    (s) => (
                      <option key={s} value={s}>
                        {s}
                      </option>
                    )
                  )}
                </Select>
              </div>
            </div>

            <div className="space-y-2">
              <div className="text-xs font-medium text-fg">Fallback / Target Chain</div>
              {steps.map((s, idx) => (
                <div key={idx} className="flex items-center gap-2">
                  <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-border bg-surface-subtle">
                    <ProviderIcon providerId={s.provider || "openai"} size={18} isDark={isDark} />
                  </div>
                  <Input
                    value={s.provider}
                    onChange={(e) =>
                      setSteps((prev) =>
                        prev.map((x, i) => (i === idx ? { ...x, provider: e.target.value } : x))
                      )
                    }
                    placeholder="provider"
                    className="w-32 font-mono text-xs"
                  />
                  <Input
                    value={s.model}
                    onChange={(e) =>
                      setSteps((prev) =>
                        prev.map((x, i) => (i === idx ? { ...x, model: e.target.value } : x))
                      )
                    }
                    placeholder="model-id"
                    className="flex-1 font-mono text-xs"
                  />
                  <Input
                    type="number"
                    step="0.5"
                    min="0.1"
                    value={s.weight}
                    onChange={(e) =>
                      setSteps((prev) =>
                        prev.map((x, i) => (i === idx ? { ...x, weight: e.target.value } : x))
                      )
                    }
                    className="w-16 font-mono text-xs"
                    title="Weight"
                  />
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => setSteps((prev) => prev.filter((_, i) => i !== idx))}
                  >
                    <X className="h-3.5 w-3.5" />
                  </Button>
                </div>
              ))}
            </div>

            <div className="flex items-center justify-between pt-1">
              <Button
                variant="default"
                size="sm"
                onClick={() =>
                  setSteps((prev) => [...prev, { provider: "openai", model: "gpt-4o", weight: "1" }])
                }
              >
                <Plus className="h-3.5 w-3.5" />
                Add Target Step
              </Button>
              <Button variant="primary" size="sm" disabled={saving} onClick={saveCombo}>
                {saving ? "Saving..." : "Save Combo"}
              </Button>
            </div>
          </CardContent>
        </Card>

        {/* Route Simulator */}
        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <Play className="h-4 w-4 text-success" />
              <CardTitle>Route Simulator (Dry-Run)</CardTitle>
            </div>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex flex-wrap gap-2">
              <Input
                value={simModel}
                onChange={(e) => setSimModel(e.target.value)}
                placeholder="model or combo name"
                className="flex-1 font-mono text-xs"
              />
              <Select
                value={simStrat}
                onChange={(e) => setSimStrat(e.target.value)}
                className="w-40 text-xs"
              >
                <option value="">Default strategy</option>
                {strategies.map((s) => (
                  <option key={s} value={s}>
                    {s}
                  </option>
                ))}
              </Select>
              <Button variant="primary" size="sm" onClick={runSimulate}>
                Simulate
              </Button>
            </div>

            {!simResult ? (
              <EmptyState
                title="No simulation run yet"
                description="Enter any model ID or combo name to preview exact candidate order and circuit breaker health."
              />
            ) : (
              <div className="space-y-2.5">
                <div className="flex items-center gap-2">
                  <Badge variant="accent">strategy: {simResult.strategy}</Badge>
                  <Badge variant="default">
                    {(simResult.candidates || []).length} candidate(s)
                  </Badge>
                </div>
                <div className="overflow-x-auto rounded-lg border border-border">
                  <table className="w-full text-left text-xs">
                    <thead className="bg-surface-subtle text-fg-muted">
                      <tr>
                        <th className="px-3 py-2 font-medium">#</th>
                        <th className="px-3 py-2 font-medium">Target</th>
                        <th className="px-3 py-2 font-medium">Weight</th>
                        <th className="px-3 py-2 font-medium">Breaker</th>
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-border">
                      {(simResult.candidates || []).slice(0, 10).map((c: any, i: number) => (
                        <tr key={i}>
                          <td className="px-3 py-2 font-mono text-fg-muted">{i + 1}</td>
                          <td className="px-3 py-2 font-mono text-fg">
                            {c.provider}/{c.model}
                          </td>
                          <td className="px-3 py-2 font-mono">{Number(c.weight).toFixed(1)}</td>
                          <td className="px-3 py-2">
                            <Badge variant={c.breaker_open ? "danger" : "success"}>
                              {c.breaker_open ? "OPEN" : "CLOSED"}
                            </Badge>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
            )}
          </CardContent>
        </Card>
      </div>

      {/* Active Combos List */}
      <Card>
        <CardHeader>
          <CardTitle>Active Combos</CardTitle>
          <Badge variant="default">{combos.length}</Badge>
        </CardHeader>
        <CardContent>
          {combos.length === 0 ? (
            <EmptyState
              title="No combos configured yet"
              description="Create a combo above to automatically load-balance or failover between multiple providers."
            />
          ) : (
            <div className="space-y-3">
              {combos.map((c) => (
                <div
                  key={c.id}
                  className="flex flex-col justify-between gap-3 rounded-xl border border-border bg-surface-subtle/40 p-4 sm:flex-row sm:items-center"
                >
                  <div className="space-y-2">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-semibold text-fg">{c.name}</span>
                      <Badge variant="accent">{c.strategy}</Badge>
                    </div>
                    <div className="flex flex-wrap items-center gap-1.5">
                      {(c.models || []).map((m, idx) => (
                        <div key={idx} className="flex items-center gap-1.5">
                          <span className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface px-2.5 py-1 font-mono text-xs text-fg">
                            <ProviderIcon providerId={m.provider} size={14} isDark={isDark} />
                            {m.provider}/{m.model}
                          </span>
                          {idx < c.models.length - 1 && (
                            <ArrowRight className="h-3.5 w-3.5 text-fg-muted" />
                          )}
                        </div>
                      ))}
                    </div>
                  </div>
                  <Button
                    variant="danger"
                    size="sm"
                    onClick={() => delCombo(c.id)}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                    Delete
                  </Button>
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
