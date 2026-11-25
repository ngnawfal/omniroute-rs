import { useRef, useState } from "react";
import {
  Send,
  Trash2,
  Sparkles,
  Bot,
  User,
  Copy,
  SlidersHorizontal,
  Loader2,
} from "lucide-react";
import {
  Card,
  Badge,
  Button,
  Input,
  PageHeader,
} from "@/components/ui/primitives";
import { ProviderIcon } from "@/components/ProviderIcon";
import { apiPost } from "@/lib/api";
import { cn } from "@/lib/utils";

interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  content: string;
  meta?: string[];
  isError?: boolean;
}

const PRESET_MODELS = [
  { id: "pollinations/openai-fast", label: "Pollinations Fast (Free)" },
  { id: "antigravity/claude-sonnet-4-6", label: "Antigravity · Claude Sonnet 4.6" },
  { id: "antigravity/gemini-2.5-pro", label: "Antigravity · Gemini 2.5 Pro" },
  { id: "claude/claude-sonnet-4-6", label: "Claude · Sonnet 4.6" },
  { id: "codex/gpt-5.3-codex", label: "Codex · GPT-5.3" },
  { id: "openai/gpt-4o", label: "OpenAI · GPT-4o" },
  { id: "deepseek/deepseek-chat", label: "DeepSeek · V3" },
];

export function PlaygroundPage({
  isDark,
  onAfterChat,
  onToast,
}: {
  isDark: boolean;
  onAfterChat: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [model, setModel] = useState("pollinations/openai-fast");
  const [temperature, setTemperature] = useState("0.7");
  const [systemPrompt, setSystemPrompt] = useState("");
  const [showSettings, setShowSettings] = useState(false);
  const [prompt, setPrompt] = useState("");
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [sending, setSending] = useState(false);
  const feedRef = useRef<HTMLDivElement>(null);

  const activeProvider = model.includes("/") ? model.split("/")[0] : "openai";

  const scrollToBottom = () => {
    window.setTimeout(() => {
      if (feedRef.current) feedRef.current.scrollTop = feedRef.current.scrollHeight;
    }, 30);
  };

  const sendMessage = async () => {
    const text = prompt.trim();
    if (!text || !model.trim() || sending) return;
    setPrompt("");
    const userMsg: ChatMessage = {
      id: Date.now(),
      role: "user",
      content: text,
      meta: [model],
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
      model: model.trim(),
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
        description="Test routing, OAuth accounts, free-tier models, semantic caching & RTK compression directly via /v1/chat/completions."
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

      {/* Model Selector Bar */}
      <Card className="mb-4 p-3.5">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
          <div className="flex flex-1 items-center gap-2.5">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-border bg-surface-subtle">
              <ProviderIcon providerId={activeProvider} size={20} isDark={isDark} />
            </div>
            <Input
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder="provider/model-id or combo name..."
              className="font-mono text-xs"
            />
          </div>
          <div className="flex flex-wrap gap-1.5">
            {PRESET_MODELS.slice(0, 4).map((p) => (
              <button
                key={p.id}
                type="button"
                onClick={() => setModel(p.id)}
                className={cn(
                  "rounded-md border px-2.5 py-1 text-[11px] font-medium transition-colors cursor-pointer",
                  model === p.id
                    ? "border-accent bg-accent-subtle text-accent-ink"
                    : "border-border bg-surface-subtle/60 text-fg-muted hover:text-fg"
                )}
              >
                {p.label}
              </button>
            ))}
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
                <Sparkles className="h-6 w-6" />
              </div>
              <div className="text-sm font-semibold text-fg">Ready to test {model}</div>
              <p className="mt-1 max-w-md text-xs text-fg-muted">
                Send a message below to inspect real provider latency, failover routing, cache hits, and token compression headers.
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
                          onClick={() => {
                            navigator.clipboard.writeText(m.content);
                            onToast("Response copied", "info");
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
