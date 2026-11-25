import { useState } from "react";
import { ShieldCheck, KeyRound, Copy, Plus, Lock } from "lucide-react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Badge,
  Button,
  Input,
  Switch,
  SegmentedControl,
  PageHeader,
  EmptyState,
} from "@/components/ui/primitives";
import { apiPost, apiPut } from "@/lib/api";

export function SecurityPage({
  keys,
  guardrails,
  onRefresh,
  onToast,
}: {
  keys: any[];
  guardrails: any;
  onRefresh: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [label, setLabel] = useState("");
  const [rpm, setRpm] = useState("");
  const [createdKey, setCreatedKey] = useState<string | null>(null);
  const [ruleName, setRuleName] = useState("");
  const [ruleRegex, setRuleRegex] = useState("");

  const createKey = async () => {
    const body: any = { label: label.trim() || "dashboard" };
    const rNum = Number.parseInt(rpm, 10);
    if (rNum > 0) body.rpm_limit = rNum;
    const r = await apiPost("/api/keys", body);
    if (r.ok) {
      setCreatedKey(r.data.key);
      setLabel("");
      setRpm("");
      onToast("API key generated — copy it now!", "success");
      onRefresh();
    } else {
      onToast(r.error || "Failed to generate key", "error");
    }
  };

  const updateGuard = async (patch: Record<string, any>) => {
    const r = await apiPut("/api/settings", {
      guardrails_enabled: patch.guardrails_enabled ?? !!guardrails?.enabled,
      pii_redaction_enabled: patch.pii_redaction_enabled ?? !!guardrails?.pii_redaction_enabled,
      pii_vault_enabled: patch.pii_vault_enabled ?? !!guardrails?.pii_vault_enabled,
      agent_guard_mode: patch.agent_guard_mode ?? (guardrails?.agent_guard_mode || "off"),
    });
    if (r.ok) {
      onToast("Guardrail settings updated", "success");
      onRefresh();
    } else {
      onToast(r.error || "Update failed", "error");
    }
  };

  const addPattern = async () => {
    if (!ruleName.trim() || !ruleRegex.trim()) {
      onToast("Pattern name and regex are required", "error");
      return;
    }
    const r = await apiPost("/api/guardrails/patterns", {
      name: ruleName.trim(),
      regex: ruleRegex.trim(),
    });
    if (r.ok) {
      setRuleName("");
      setRuleRegex("");
      onToast("Custom guardrail rule added", "success");
      onRefresh();
    } else {
      onToast(r.error || "Invalid regex pattern", "error");
    }
  };

  const events = guardrails?.recent_events || [];

  return (
    <div className="space-y-6">
      <PageHeader
        title="API Keys & Guardrails"
        description="Manage gateway Bearer keys with RPM limits, prompt-injection defense, reversible PII vault & agent-guard."
      />

      <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
        {/* Gateway API Keys */}
        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <KeyRound className="h-4 w-4 text-accent-ink" />
              <CardTitle>Gateway API Keys</CardTitle>
            </div>
            <Badge variant={keys.length ? "accent" : "default"}>
              {keys.length ? `${keys.length} strict` : "Open local mode"}
            </Badge>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex flex-wrap gap-2">
              <Input
                value={label}
                onChange={(e) => setLabel(e.target.value)}
                placeholder="Key label (e.g. cursor-dev)"
                className="flex-1"
              />
              <Input
                type="number"
                value={rpm}
                onChange={(e) => setRpm(e.target.value)}
                placeholder="RPM"
                className="w-24"
              />
              <Button variant="primary" size="sm" onClick={createKey}>
                <Plus className="h-3.5 w-3.5" />
                Generate
              </Button>
            </div>

            {createdKey && (
              <div className="flex items-center justify-between gap-2 rounded-xl border border-accent/40 bg-accent-subtle px-3.5 py-2.5">
                <code className="truncate font-mono text-xs text-fg">{createdKey}</code>
                <Button
                  variant="primary"
                  size="sm"
                  onClick={() => {
                    navigator.clipboard.writeText(createdKey);
                    onToast("API key copied", "info");
                  }}
                >
                  <Copy className="h-3.5 w-3.5" /> Copy
                </Button>
              </div>
            )}

            {keys.length === 0 ? (
              <EmptyState
                icon={<Lock className="h-5 w-5" />}
                title="Open-door local mode"
                description="Generate your first API key above to enforce strict Bearer authentication on /v1/* endpoints."
              />
            ) : (
              <div className="overflow-x-auto rounded-lg border border-border">
                <table className="w-full text-left text-xs">
                  <thead className="bg-surface-subtle text-fg-muted">
                    <tr>
                      <th className="px-3 py-2 font-medium">Preview</th>
                      <th className="px-3 py-2 font-medium">Label</th>
                      <th className="px-3 py-2 font-medium">RPM</th>
                      <th className="px-3 py-2 font-medium">Status</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-border">
                    {keys.map((k: any, i: number) => (
                      <tr key={i}>
                        <td className="px-3 py-2 font-mono">{k.key_preview}</td>
                        <td className="px-3 py-2 font-medium text-fg">{k.label || "—"}</td>
                        <td className="px-3 py-2 font-mono">{k.rpm_limit ?? "∞"}</td>
                        <td className="px-3 py-2">
                          <Badge variant={k.disabled ? "danger" : "success"}>
                            {k.disabled ? "disabled" : "active"}
                          </Badge>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </CardContent>
        </Card>

        {/* Guardrails & Agent-Guard */}
        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <ShieldCheck className="h-4 w-4 text-success" />
              <CardTitle>Guardrails &amp; Agent-Guard</CardTitle>
            </div>
            <Badge variant="default">
              {guardrails?.builtin_injection_patterns || 0} inj · {guardrails?.builtin_pii_patterns || 0} PII
            </Badge>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="divide-y divide-border rounded-xl border border-border bg-surface-subtle/35 px-4">
              <div className="flex items-center justify-between gap-4 py-3">
                <div>
                  <div className="text-xs font-semibold text-fg">Prompt-Injection Defense</div>
                  <div className="text-[11px] text-fg-muted">Blocks override &amp; jailbreak payloads before dispatch</div>
                </div>
                <Switch
                  checked={!!guardrails?.enabled}
                  onChange={(v) => updateGuard({ guardrails_enabled: v })}
                />
              </div>
              <div className="flex items-center justify-between gap-4 py-3">
                <div>
                  <div className="text-xs font-semibold text-fg">PII Masking</div>
                  <div className="text-[11px] text-fg-muted">Redacts emails, SSNs, tokens &amp; phone numbers</div>
                </div>
                <Switch
                  checked={!!guardrails?.pii_redaction_enabled}
                  onChange={(v) => updateGuard({ pii_redaction_enabled: v })}
                />
              </div>
              <div className="flex items-center justify-between gap-4 py-3">
                <div>
                  <div className="text-xs font-semibold text-fg">Reversible PII Vault</div>
                  <div className="text-[11px] text-fg-muted">Automatically restores masked values in responses</div>
                </div>
                <Switch
                  checked={!!guardrails?.pii_vault_enabled}
                  onChange={(v) => updateGuard({ pii_vault_enabled: v })}
                />
              </div>
              <div className="flex flex-wrap items-center justify-between gap-3 py-3">
                <div>
                  <div className="text-xs font-semibold text-fg">Agent-Guard Mode</div>
                  <div className="text-[11px] text-fg-muted">Monitors or blocks anomalous tool loops</div>
                </div>
                <SegmentedControl
                  value={(guardrails?.agent_guard_mode || "off") as "off" | "monitor" | "block"}
                  onChange={(v) => updateGuard({ agent_guard_mode: v })}
                  options={[
                    { value: "off", label: "Off" },
                    { value: "monitor", label: "Monitor" },
                    { value: "block", label: "Block" },
                  ]}
                />
              </div>
            </div>

            <div className="flex flex-wrap gap-2">
              <Input
                value={ruleName}
                onChange={(e) => setRuleName(e.target.value)}
                placeholder="Rule name"
                className="w-36"
              />
              <Input
                value={ruleRegex}
                onChange={(e) => setRuleRegex(e.target.value)}
                placeholder="Custom regex pattern..."
                className="flex-1 font-mono text-xs"
              />
              <Button variant="default" size="sm" onClick={addPattern}>
                Add Rule
              </Button>
            </div>
          </CardContent>
        </Card>
      </div>

      {/* Recent Audit Events */}
      <Card>
        <CardHeader>
          <CardTitle>Recent Guardrail Audit Events</CardTitle>
          <Badge variant="default">{events.length} recorded</Badge>
        </CardHeader>
        <CardContent>
          {events.length === 0 ? (
            <EmptyState
              title="No guardrail events recorded"
              description="Blocked prompt injections, PII redactions, and agent-guard alerts will appear here."
            />
          ) : (
            <div className="overflow-x-auto rounded-lg border border-border">
              <table className="w-full text-left text-xs">
                <thead className="bg-surface-subtle text-fg-muted">
                  <tr>
                    <th className="px-3 py-2 font-medium">Kind</th>
                    <th className="px-3 py-2 font-medium">Action</th>
                    <th className="px-3 py-2 font-medium">Provider / Model</th>
                    <th className="px-3 py-2 font-medium">Time</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-border">
                  {events.slice(0, 15).map((e: any, i: number) => (
                    <tr key={i}>
                      <td className="px-3 py-2 font-mono">{e.kind}</td>
                      <td className="px-3 py-2">
                        <Badge variant={e.action === "block" ? "danger" : "warning"}>
                          {e.action}
                        </Badge>
                      </td>
                      <td className="px-3 py-2 font-mono">
                        {e.provider} {e.model}
                      </td>
                      <td className="px-3 py-2 font-mono text-fg-muted">
                        {new Date(e.ts).toLocaleTimeString()}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
