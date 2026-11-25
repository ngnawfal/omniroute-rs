import { useEffect, useState } from "react";
import { Network, Play, Square, RefreshCw, Download, Shield } from "lucide-react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Badge,
  Button,
  Input,
  PageHeader,
} from "@/components/ui/primitives";
import { ProviderIcon } from "@/components/ProviderIcon";
import { apiPost, apiPut } from "@/lib/api";

export function MitmPage({
  mitm,
  isDark,
  onRefresh,
  onToast,
}: {
  mitm: any;
  isDark: boolean;
  onRefresh: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [port, setPort] = useState("20443");
  const [bypass, setBypass] = useState("");

  useEffect(() => {
    if (Array.isArray(mitm?.bypass_patterns)) {
      setBypass(mitm.bypass_patterns.join(", "));
    }
  }, [mitm?.bypass_patterns?.length]);

  const running = !!mitm?.running;
  const targets = mitm?.targets || [];

  const start = async () => {
    const r = await apiPost("/api/mitm/start", { port: Number.parseInt(port, 10) || 20443 });
    if (r.ok) {
      onToast("MITM TLS proxy started", "success");
      onRefresh();
    } else onToast(r.error || "Start failed", "error");
  };

  const stop = async () => {
    await apiPost("/api/mitm/stop", {});
    onToast("MITM proxy stopped", "info");
    onRefresh();
  };

  const rotateCa = async () => {
    await apiPost("/api/mitm/regenerate-cert", {});
    onToast("Rotated ECDSA P-256 Root CA", "success");
    onRefresh();
  };

  const saveBypass = async () => {
    const patterns = bypass
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    const r = await apiPut("/api/mitm/bypass", { patterns });
    if (r.ok) {
      onToast("Updated TLS bypass rules", "success");
      onRefresh();
    } else onToast(r.error || "Failed", "error");
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="MITM TLS Proxy"
        description="Pure-Rust ECDSA P-256 Root CA and dynamic leaf certificate HTTPS CONNECT interception for 10 AI coding agents (zero OpenSSL)."
        badge={
          <Badge variant={running ? "success" : "default"}>
            {running ? `LIVE · ${mitm?.bind_addr}` : "STOPPED"}
          </Badge>
        }
        actions={
          <>
            <Input
              type="number"
              value={port}
              onChange={(e) => setPort(e.target.value)}
              className="h-8 w-24 font-mono text-xs"
            />
            {!running ? (
              <Button variant="primary" size="sm" onClick={start}>
                <Play className="h-3.5 w-3.5" /> Start Proxy
              </Button>
            ) : (
              <Button variant="danger" size="sm" onClick={stop}>
                <Square className="h-3.5 w-3.5" /> Stop
              </Button>
            )}
            <Button variant="default" size="sm" onClick={rotateCa}>
              <RefreshCw className="h-3.5 w-3.5" /> Rotate CA
            </Button>
            <a
              href="/api/mitm/ca.crt"
              download="omniroute-mitm-ca.crt"
              className="inline-flex h-8 items-center gap-1.5 rounded-md border border-border bg-surface-subtle px-3 text-xs font-medium text-fg hover:bg-surface-muted"
            >
              <Download className="h-3.5 w-3.5" /> Download CA .crt
            </a>
          </>
        }
      />

      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        <Card className="p-4">
          <div className="text-xs text-fg-muted">Proxy State</div>
          <div className="mt-1.5 text-xl font-semibold text-fg">
            {running ? "RUNNING" : "STOPPED"}
          </div>
          <div className="mt-1 font-mono text-[11px] text-fg-muted">
            {mitm?.bind_addr || `127.0.0.1:${port}`}
          </div>
        </Card>
        <Card className="p-4">
          <div className="text-xs text-fg-muted">Intercepted Connections</div>
          <div className="mt-1.5 font-mono text-xl font-semibold text-fg">
            {mitm?.metrics?.intercepted ?? 0}
          </div>
          <div className="mt-1 text-[11px] text-fg-muted">
            Total: {mitm?.metrics?.total_connections ?? 0}
          </div>
        </Card>
        <Card className="p-4">
          <div className="text-xs text-fg-muted">Tunnel Bypassed</div>
          <div className="mt-1.5 font-mono text-xl font-semibold text-fg">
            {mitm?.metrics?.bypassed ?? 0}
          </div>
          <div className="mt-1 text-[11px] text-fg-muted">
            {mitm?.bypass_patterns?.length ?? 0} bypass rules
          </div>
        </Card>
        <Card className="p-4">
          <div className="text-xs text-fg-muted">Registered Agents</div>
          <div className="mt-1.5 font-mono text-xl font-semibold text-fg">{targets.length}</div>
          <div className="mt-1 text-[11px] text-fg-muted">ECDSA P-256 CA</div>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <Shield className="h-4 w-4 text-accent-ink" />
            <CardTitle>CA Fingerprint &amp; Host Bypass Rules</CardTitle>
          </div>
          <Badge variant="outline" className="font-mono">
            {mitm?.ca_fingerprint_sha256 ? String(mitm.ca_fingerprint_sha256).slice(0, 28) + "…" : "—"}
          </Badge>
        </CardHeader>
        <CardContent>
          <div className="flex flex-wrap gap-2">
            <Input
              value={bypass}
              onChange={(e) => setBypass(e.target.value)}
              placeholder="Comma-separated hosts to bypass (e.g. api.anthropic.com, chatgpt.com)"
              className="flex-1 font-mono text-xs"
            />
            <Button variant="primary" size="sm" onClick={saveBypass}>
              Update Bypass
            </Button>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <Network className="h-4 w-4 text-accent-ink" />
            <CardTitle>Intercept Target Registry ({targets.length} agents)</CardTitle>
          </div>
        </CardHeader>
        <CardContent>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {targets.map((t: any) => (
              <div
                key={t.id}
                className="flex items-center gap-3 rounded-xl border border-border bg-surface-subtle/40 p-3.5"
              >
                <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-border bg-surface">
                  <ProviderIcon providerId={t.id} size={20} isDark={isDark} />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-xs font-semibold text-fg">{t.name}</div>
                  <div className="truncate font-mono text-[10px] text-fg-muted">
                    {(t.hosts || []).join(", ")}
                  </div>
                </div>
                <Badge variant="default">{t.protocol || "https"}</Badge>
              </div>
            ))}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
