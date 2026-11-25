import { useState } from "react";
import { Bot, Play, ExternalLink } from "lucide-react";
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
import { apiPost } from "@/lib/api";

const SKILLS = [
  "health-report",
  "smart-routing",
  "cost-analysis",
  "quota-management",
  "provider-discovery",
  "list-capabilities",
];

export function A2APage({
  tasks,
  onRefresh,
  onToast,
}: {
  tasks: any[];
  onRefresh: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [skill, setSkill] = useState("health-report");
  const [ver, setVer] = useState<"v0.3" | "v1.0">("v0.3");
  const [msg, setMsg] = useState("Generate a full gateway health summary");
  const [result, setResult] = useState<any>(null);
  const [running, setRunning] = useState(false);

  const runSkill = async () => {
    setRunning(true);
    const method = ver === "v1.0" ? "SendMessage" : "message/send";
    const r = await apiPost("/a2a", {
      jsonrpc: "2.0",
      id: Date.now(),
      method,
      params: {
        skill,
        message: { role: "user", parts: [{ type: "text", text: msg }] },
      },
    });
    setRunning(false);
    if (r.ok) {
      setResult(r.data);
      onToast(`A2A skill ${skill} completed`, "success");
      onRefresh();
    } else {
      onToast(r.error || "A2A invoke failed", "error");
    }
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="A2A Protocol (v0.3 + v1.0)"
        description="Agent-to-Agent discovery card and owner-isolated JSON-RPC 2.0 task execution."
        actions={
          <a
            href="/.well-known/agent.json"
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex h-8 items-center gap-1.5 rounded-md border border-border bg-surface-subtle px-3 text-xs font-medium text-fg hover:bg-surface-muted"
          >
            <ExternalLink className="h-3.5 w-3.5" />
            Agent Card JSON
          </a>
        }
      />

      <div className="grid grid-cols-1 gap-5 lg:grid-cols-5">
        <Card className="lg:col-span-3">
          <CardHeader>
            <div className="flex items-center gap-2">
              <Bot className="h-4 w-4 text-accent-ink" />
              <CardTitle>Invoke A2A Skill (POST /a2a)</CardTitle>
            </div>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex flex-wrap gap-2">
              <Select value={skill} onChange={(e) => setSkill(e.target.value)} className="flex-1">
                {SKILLS.map((s) => (
                  <option key={s} value={s}>
                    {s}
                  </option>
                ))}
              </Select>
              <Select
                value={ver}
                onChange={(e) => setVer(e.target.value as "v0.3" | "v1.0")}
                className="w-28"
              >
                <option value="v0.3">A2A v0.3</option>
                <option value="v1.0">A2A v1.0</option>
              </Select>
              <Button variant="primary" size="sm" disabled={running} onClick={runSkill}>
                <Play className="h-3.5 w-3.5" />
                {running ? "Running..." : "Run Skill"}
              </Button>
            </div>

            <Input value={msg} onChange={(e) => setMsg(e.target.value)} />

            {!result ? (
              <EmptyState
                title="Ready to execute A2A task"
                description="Select any built-in skill above and run a live JSON-RPC 2.0 call."
              />
            ) : (
              <pre className="max-h-72 overflow-auto rounded-xl border border-border bg-[#0d100e] p-4 font-mono text-xs text-[#eef2ee]">
                {JSON.stringify(result, null, 2)}
              </pre>
            )}
          </CardContent>
        </Card>

        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle>Task History</CardTitle>
            <Badge variant="default">{tasks.length}</Badge>
          </CardHeader>
          <CardContent>
            {tasks.length === 0 ? (
              <EmptyState title="No A2A tasks yet" description="Executed tasks will be logged here." />
            ) : (
              <div className="space-y-2">
                {tasks.slice(0, 12).map((t: any) => (
                  <div
                    key={t.id}
                    className="flex items-center justify-between gap-2 rounded-lg border border-border bg-surface-subtle/40 px-3 py-2 text-xs"
                  >
                    <div className="min-w-0">
                      <div className="truncate font-semibold text-fg">{t.skill || "task"}</div>
                      <div className="truncate font-mono text-[10px] text-fg-muted">{t.id}</div>
                    </div>
                    <Badge variant={String(t.state).includes("COMPLETE") ? "success" : "info"}>
                      {t.state}
                    </Badge>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
