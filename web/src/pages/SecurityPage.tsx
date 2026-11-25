import { useCallback, useEffect, useState } from "react";
import { ShieldCheck, KeyRound, Copy, Plus, Lock, Bell, Send, Unlock } from "lucide-react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Badge,
  Button,
  Input,
  Switch,
  SegmentedControl,
  PageHeader,
  EmptyState,
} from "@/components/ui/primitives";
import { api, apiDelete, apiPost, apiPut } from "@/lib/api";

export function SecurityPage({
  keys,
  guardrails,
  dashboardLocked,
  onAuthChanged,
  onRefresh,
  onToast,
}: {
  keys: any[];
  guardrails: any;
  dashboardLocked?: boolean;
  onAuthChanged?: () => void;
  onRefresh: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [label, setLabel] = useState("");
  const [rpm, setRpm] = useState("");
  const [createdKey, setCreatedKey] = useState<string | null>(null);
  const [ruleName, setRuleName] = useState("");
  const [ruleRegex, setRuleRegex] = useState("");

  // Dashboard password lock state
  const [dashPassword, setDashPassword] = useState("");
  const [savingAuth, setSavingAuth] = useState(false);

  // Webhook & Telegram alerting state
  const [alertEnabled, setAlertEnabled] = useState(false);
  const [webhookUrl, setWebhookUrl] = useState("");
  const [webhookSecret, setWebhookSecret] = useState("");
  const [tgToken, setTgToken] = useState("");
  const [tgChatId, setTgChatId] = useState("");
  const [onReauth, setOnReauth] = useState(true);
  const [onBreaker, setOnBreaker] = useState(true);
  const [onBudget, setOnBudget] = useState(true);
  const [testingAlert, setTestingAlert] = useState(false);

  const loadAlerts = useCallback(async () => {
    const r = await api<any>("/api/alerts/config");
    if (r.ok && r.data?.config) {
      const c = r.data.config;
      setAlertEnabled(!!c.enabled);
      setWebhookUrl(c.webhook_url || "");
      setWebhookSecret(c.webhook_secret || "");
      setTgToken(c.telegram_bot_token || "");
      setTgChatId(c.telegram_chat_id || "");
      setOnReauth(c.on_needs_reauth !== false);
      setOnBreaker(c.on_breaker_open !== false);
      setOnBudget(c.on_budget_alert !== false);
    }
  }, []);

  useEffect(() => {
    void loadAlerts();
  }, [loadAlerts]);

  const setDashboardPassword = async () => {
    if (dashPassword.trim().length < 4) {
      onToast("Password must be at least 4 characters", "error");
      return;
    }
    setSavingAuth(true);
    const r = await apiPost("/api/auth/setup", { password: dashPassword.trim() });
    setSavingAuth(false);
    if (r.ok) {
      setDashPassword("");
      onToast("Dashboard password lock enabled (session cookie active)", "success");
      onAuthChanged?.();
      onRefresh();
    } else {
      onToast(r.error || "Failed to set password", "error");
    }
  };

  const disableDashboardPassword = async () => {
    const r = await apiDelete("/api/auth/password");
    if (r.ok) {
      onToast("Dashboard password lock disabled", "info");
      onAuthChanged?.();
      onRefresh();
    } else {
      onToast(r.error || "Failed to disable password lock", "error");
    }
  };

  const saveAlerts = async () => {
    const r = await apiPut("/api/alerts/config", {
      enabled: alertEnabled,
      webhook_url: webhookUrl.trim(),
      webhook_secret: webhookSecret.trim(),
      telegram_bot_token: tgToken.trim(),
      telegram_chat_id: tgChatId.trim(),
      on_needs_reauth: onReauth,
      on_breaker_open: onBreaker,
      on_budget_alert: onBudget,
    });
    if (r.ok) {
      onToast("Webhook & Telegram alert settings saved", "success");
      void loadAlerts();
      onRefresh();
    } else {
      onToast(r.error || "Failed to save alert settings", "error");
    }
  };

  const sendTestAlert = async () => {
    setTestingAlert(true);
    const r = await apiPost("/api/alerts/test", {});
    setTestingAlert(false);
    if (r.ok) {
      onToast("Test alert dispatched! Check your Webhook / Telegram.", "success");
    } else {
      onToast(r.error || "Test alert failed", "error");
    }
  };

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

      {/* Dashboard Password Auth + Webhook & Telegram Alerting */}
      <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <Lock className="h-4 w-4 text-accent-ink" />
              <div>
                <CardTitle>Dashboard Password Lock</CardTitle>
                <CardDescription>
                  Protect the Web UI &amp; <code className="font-mono">/api/*</code> on LAN/public networks with signed HttpOnly session cookies
                </CardDescription>
              </div>
            </div>
            <Badge variant={dashboardLocked ? "success" : "default"}>
              {dashboardLocked ? "Locked" : "Unlocked"}
            </Badge>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex flex-wrap gap-2">
              <Input
                type="password"
                value={dashPassword}
                onChange={(e) => setDashPassword(e.target.value)}
                placeholder={dashboardLocked ? "New password (min 4 chars)..." : "Set dashboard password..."}
                className="flex-1"
              />
              <Button variant="primary" size="sm" disabled={savingAuth} onClick={setDashboardPassword}>
                <Lock className="h-3.5 w-3.5" />
                {dashboardLocked ? "Update Password" : "Enable Lock"}
              </Button>
              {dashboardLocked && (
                <Button variant="danger" size="sm" onClick={disableDashboardPassword}>
                  <Unlock className="h-3.5 w-3.5" /> Disable
                </Button>
              )}
            </div>
            <p className="text-[11px] text-fg-muted">
              Inference endpoints (<code className="font-mono">/v1/*</code>) remain controlled independently by your Gateway API Keys.
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <Bell className="h-4 w-4 text-accent-ink" />
              <div>
                <CardTitle>Webhook &amp; Telegram Alerting</CardTitle>
                <CardDescription>
                  Instant push notifications on OAuth <code className="font-mono">needs_reauth</code>, circuit breakers &amp; budget caps
                </CardDescription>
              </div>
            </div>
            <Switch checked={alertEnabled} onChange={setAlertEnabled} />
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
              <Input
                value={webhookUrl}
                onChange={(e) => setWebhookUrl(e.target.value)}
                placeholder="Webhook URL (https://...)"
                className="sm:col-span-2 font-mono text-xs"
              />
              <Input
                type="password"
                value={webhookSecret}
                onChange={(e) => setWebhookSecret(e.target.value)}
                placeholder="HMAC secret (opt)"
                className="font-mono text-xs"
              />
            </div>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              <Input
                type="password"
                value={tgToken}
                onChange={(e) => setTgToken(e.target.value)}
                placeholder="Telegram Bot Token"
                className="font-mono text-xs"
              />
              <Input
                value={tgChatId}
                onChange={(e) => setTgChatId(e.target.value)}
                placeholder="Telegram Chat ID"
                className="font-mono text-xs"
              />
            </div>
            <div className="flex flex-wrap items-center justify-between gap-2 pt-1">
              <div className="flex flex-wrap items-center gap-3 text-xs text-fg-muted">
                <label className="flex items-center gap-1.5 cursor-pointer">
                  <input type="checkbox" checked={onReauth} onChange={(e) => setOnReauth(e.target.checked)} />
                  <span>OAuth re-auth</span>
                </label>
                <label className="flex items-center gap-1.5 cursor-pointer">
                  <input type="checkbox" checked={onBreaker} onChange={(e) => setOnBreaker(e.target.checked)} />
                  <span>Breaker open</span>
                </label>
                <label className="flex items-center gap-1.5 cursor-pointer">
                  <input type="checkbox" checked={onBudget} onChange={(e) => setOnBudget(e.target.checked)} />
                  <span>Budget caps</span>
                </label>
              </div>
              <div className="flex gap-2">
                <Button variant="default" size="sm" disabled={testingAlert} onClick={sendTestAlert}>
                  <Send className="h-3.5 w-3.5" />
                  {testingAlert ? "Sending..." : "Test"}
                </Button>
                <Button variant="primary" size="sm" onClick={saveAlerts}>
                  Save Alerts
                </Button>
              </div>
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
