import { useEffect, useRef, useState } from "react";
import {
  ExternalLink,
  Copy,
  CheckCircle2,
  AlertTriangle,
  Loader2,
  RefreshCw,
} from "lucide-react";
import { Dialog, Button, Badge, Input, SegmentedControl } from "./ui/primitives";
import { ProviderIcon } from "./ProviderIcon";
import { apiPost } from "@/lib/api";

export interface OAuthLaunchRequest {
  providerId: string;
  displayName: string;
  popup?: Window | null;
}

export function computeOAuthRedirectUri(prov: string): string {
  const port = window.location.port || (window.location.protocol === "https:" ? "443" : "80");
  const isLocal = isTrueLocalhost();
  if (prov === "claude") return "https://platform.claude.com/oauth/code/callback";
  if (prov === "codex" || prov === "chatgpt") return "http://localhost:1455/auth/callback";
  if (prov === "xai-oauth") return "http://127.0.0.1:56121/callback";
  if (prov === "grok-cli") return "http://127.0.0.1:56122/callback";
  if (prov === "openference") return "http://127.0.0.1:56123/callback";
  if (prov === "devin-desktop") return "http://127.0.0.1:59653/callback";
  if (prov === "antigravity" || prov === "agy" || prov === "gemini-cli") {
    return `http://127.0.0.1:${port}/callback`;
  }
  if (prov === "renderhour") {
    const host = isLocal ? window.location.hostname : "localhost";
    return `http://${host}:${port}/callback`;
  }
  return `${window.location.origin}/callback`;
}

export function isTrueLocalhost(): boolean {
  const h = window.location.hostname;
  return h === "localhost" || h === "127.0.0.1" || h === "::1" || h === "[::1]";
}

/**
 * Synchronously opens a popup right inside the user click handler so browser popup
 * blockers never suppress the redirect to Google / Claude / OpenAI / GitHub.
 */
export function openOAuthPopupSync(displayName: string): Window | null {
  try {
    const w = window.open(
      "about:blank",
      "omniroute_oauth_login",
      "width=620,height=740,scrollbars=yes,resizable=yes"
    );
    if (w && w.document) {
      w.document.title = `Redirecting to ${displayName}...`;
      w.document.body.innerHTML = `<div style="font-family:system-ui,sans-serif;background:#11120f;color:#f2f0e8;min-height:100vh;margin:0;display:flex;align-items:center;justify-content:center;font-size:15px">Redirecting to ${displayName.replace(/[<>&"]/g, "")} login page...</div>`;
    }
    return w;
  } catch {
    return null;
  }
}

export function OAuthModal({
  request,
  isDark,
  onClose,
  onSuccess,
  onToast,
}: {
  request: OAuthLaunchRequest | null;
  isDark: boolean;
  onClose: () => void;
  onSuccess: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [tab, setTab] = useState<"browser" | "paste">("browser");
  const [step, setStep] = useState<"loading" | "pkce" | "device" | "import_only" | "success" | "error">("loading");
  const [session, setSession] = useState<any>(null);
  const [cbInput, setCbInput] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [errorMsg, setErrorMsg] = useState("");
  const [successInfo, setSuccessInfo] = useState<any>(null);
  const [openedPopup, setOpenedPopup] = useState(false);
  const [remainingSec, setRemainingSec] = useState<number>(0);

  const [pasteAt, setPasteAt] = useState("");
  const [pasteRt, setPasteRt] = useState("");

  const processedRef = useRef(false);
  const pollRef = useRef<number | null>(null);
  const countdownRef = useRef<number | null>(null);

  const stopTimers = () => {
    if (pollRef.current) {
      window.clearInterval(pollRef.current);
      pollRef.current = null;
    }
    if (countdownRef.current) {
      window.clearInterval(countdownRef.current);
      countdownRef.current = null;
    }
  };

  const completeCallback = async (providerId: string, state: string, rawCode: string) => {
    const trimmed = rawCode.trim();
    if (!trimmed) {
      onToast("Paste the full callback URL or authorization code first", "error");
      return;
    }
    setSubmitting(true);
    const res = await apiPost(`/api/oauth/${encodeURIComponent(providerId)}/callback`, {
      code: trimmed,
      state,
    });
    setSubmitting(false);
    if (res.ok) {
      processedRef.current = true;
      stopTimers();
      setSuccessInfo(res.data);
      setStep("success");
      const who = res.data?.email || res.data?.label || res.data?.connection_id || providerId;
      onToast(`Connected ${providerId} (${who})`, "success");
      onSuccess();
    } else {
      processedRef.current = false;
      setErrorMsg(res.error || "Token exchange failed");
      setStep("error");
    }
  };

  // Start flow whenever a new request opens
  useEffect(() => {
    if (!request) {
      stopTimers();
      return;
    }
    let cancelled = false;
    processedRef.current = false;
    stopTimers();
    setTab("browser");
    setStep("loading");
    setSession(null);
    setCbInput("");
    setErrorMsg("");
    setSuccessInfo(null);
    setOpenedPopup(false);
    setPasteAt("");
    setPasteRt("");

    const run = async () => {
      const redirectUri = computeOAuthRedirectUri(request.providerId);
      const res = await apiPost(
        `/api/oauth/${encodeURIComponent(request.providerId)}/authorize`,
        { redirect_uri: redirectUri }
      );
      if (cancelled) return;
      if (!res.ok) {
        if (request.popup && !request.popup.closed) {
          try { request.popup.close(); } catch {}
        }
        setErrorMsg(res.error || `HTTP ${res.status}`);
        setStep("error");
        return;
      }

      const d = res.data;
      setSession(d);

      if (d.flow === "pkce") {
        const authUrl = d.authorize_url || d.authUrl;
        let didOpen = false;
        if (request.popup && !request.popup.closed && authUrl) {
          try {
            request.popup.location.href = authUrl;
            request.popup.focus();
            didOpen = true;
          } catch {}
        }
        if (!didOpen && authUrl) {
          try {
            const w = window.open(authUrl, "omniroute_oauth_login");
            if (w) didOpen = true;
          } catch {}
        }
        setOpenedPopup(didOpen);
        setStep("pkce");
      } else if (d.flow === "device_code") {
        const verifyUrl = d.verification_uri_complete || d.verification_uri;
        if (request.popup && !request.popup.closed && verifyUrl) {
          try {
            request.popup.location.href = verifyUrl;
            request.popup.focus();
            setOpenedPopup(true);
          } catch {}
        }
        const expSec = Math.max(60, Number(d.expires_in) || 900);
        const intervalMs = Math.max(3, Number(d.interval) || 5) * 1000;
        const deadline = Date.now() + expSec * 1000;
        setRemainingSec(expSec);
        setStep("device");

        countdownRef.current = window.setInterval(() => {
          const rem = Math.max(0, Math.ceil((deadline - Date.now()) / 1000));
          setRemainingSec(rem);
          if (rem <= 0) {
            stopTimers();
            setErrorMsg("Device authorization timed out. Please try again.");
            setStep("error");
          }
        }, 1000);

        pollRef.current = window.setInterval(async () => {
          if (processedRef.current) return;
          const pr = await apiPost(`/api/oauth/${encodeURIComponent(d.provider)}/poll`, {
            state: d.state,
          });
          if (pr.status === 201 || pr.data?.status === "authorized" || pr.data?.success) {
            processedRef.current = true;
            stopTimers();
            setSuccessInfo(pr.data);
            setStep("success");
            onToast(`Connected ${d.provider} (${pr.data?.label || "authorized"})`, "success");
            onSuccess();
          } else if (pr.status === 410 || pr.status === 403) {
            processedRef.current = true;
            stopTimers();
            setErrorMsg(pr.data?.error || pr.data?.status || "Authorization denied or expired");
            setStep("error");
          }
        }, intervalMs);
      } else {
        if (request.popup && !request.popup.closed) {
          try { request.popup.close(); } catch {}
        }
        setStep("import_only");
      }
    };

    void run();
    return () => {
      cancelled = true;
      stopTimers();
    };
  }, [request]);

  // Listen to 3-channel `/callback` auto-relay (postMessage + BroadcastChannel + localStorage)
  useEffect(() => {
    if (!request || !session || step !== "pkce") return;
    const handleRelay = (payload: any) => {
      if (!payload || processedRef.current) return;
      if (payload.state && session.state && payload.state !== session.state) return;
      if (payload.error) {
        processedRef.current = true;
        setErrorMsg(payload.errorDescription || payload.error);
        setStep("error");
        return;
      }
      if (payload.code) {
        processedRef.current = true;
        void completeCallback(session.provider, session.state, payload.fullUrl || payload.code);
      }
    };

    const onMsg = (ev: MessageEvent) => {
      if (ev.data?.type === "oauth_callback" && ev.data?.data) handleRelay(ev.data.data);
    };
    const onStorage = (ev: StorageEvent) => {
      if (ev.key === "oauth_callback" && ev.newValue) {
        try {
          const parsed = JSON.parse(ev.newValue);
          localStorage.removeItem("oauth_callback");
          handleRelay(parsed);
        } catch {}
      }
    };

    window.addEventListener("message", onMsg);
    window.addEventListener("storage", onStorage);
    let bc: BroadcastChannel | null = null;
    try {
      bc = new BroadcastChannel("oauth_callback");
      bc.onmessage = (ev) => handleRelay(ev.data);
    } catch {}

    return () => {
      window.removeEventListener("message", onMsg);
      window.removeEventListener("storage", onStorage);
      if (bc) bc.close();
    };
  }, [request, session, step]);

  const copy = async (text: string, label = "Copied to clipboard") => {
    try {
      await navigator.clipboard.writeText(text);
      onToast(label, "info");
    } catch {
      onToast("Copy failed — select manually", "error");
    }
  };

  const handlePasteImport = async () => {
    if (!request || !pasteAt.trim()) {
      onToast("Paste an access token first", "error");
      return;
    }
    setSubmitting(true);
    const res = await apiPost(`/api/oauth/${encodeURIComponent(request.providerId)}/import`, {
      access_token: pasteAt.trim(),
      refresh_token: pasteRt.trim() || undefined,
    });
    setSubmitting(false);
    if (res.ok) {
      setSuccessInfo({ provider: request.providerId, connection_id: res.data?.connection_id || "imported" });
      setStep("success");
      onToast(`Token imported for ${request.providerId}`, "success");
      onSuccess();
    } else {
      onToast(res.error || "Import failed", "error");
    }
  };

  if (!request) return null;
  const isLocal = isTrueLocalhost();
  const isLanLoopback =
    !isLocal &&
    ["antigravity", "agy", "gemini-cli", "codex", "chatgpt", "xai-oauth", "grok-cli"].includes(
      request.providerId
    );
  const authUrl = session?.authorize_url || session?.authUrl || "";
  const verifyUrl = session?.verification_uri_complete || session?.verification_uri || "";

  return (
    <Dialog
      open={!!request}
      onClose={onClose}
      title={`Connect ${request.displayName}`}
      description={
        step === "pkce"
          ? openedPopup
            ? "Provider login page opened in a new window"
            : "Click Open Login Page below to authenticate"
          : step === "device"
            ? "Enter the device code on the opened verification page"
            : step === "success"
              ? "Account linked and ready to route inference traffic"
              : "Official OAuth 2.0 authentication"
      }
      icon={<ProviderIcon providerId={request.providerId} size={24} isDark={isDark} />}
      maxWidth="max-w-lg"
    >
      {step !== "success" && (
        <div className="mb-4">
          <SegmentedControl
            value={tab}
            onChange={setTab}
            options={[
              { value: "browser", label: "Browser Login" },
              { value: "paste", label: "Paste Token" },
            ]}
          />
        </div>
      )}

      {tab === "paste" && step !== "success" ? (
        <div className="space-y-3">
          <p className="text-xs leading-relaxed text-fg-muted">
            Import an existing OAuth access token, Claude setup token, or IDE session token directly.
          </p>
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-fg">Access / Setup Token</label>
            <Input
              type="password"
              value={pasteAt}
              onChange={(e) => setPasteAt(e.target.value)}
              placeholder="Paste token..."
            />
          </div>
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-fg">Refresh Token (optional)</label>
            <Input
              type="password"
              value={pasteRt}
              onChange={(e) => setPasteRt(e.target.value)}
              placeholder="Paste refresh token (optional)..."
            />
          </div>
          <div className="flex justify-end gap-2 pt-2">
            <Button variant="ghost" onClick={onClose}>Cancel</Button>
            <Button variant="primary" disabled={submitting} onClick={handlePasteImport}>
              {submitting ? "Saving..." : "Save Token"}
            </Button>
          </div>
        </div>
      ) : step === "loading" ? (
        <div className="flex flex-col items-center gap-3 py-8 text-center">
          <Loader2 className="h-7 w-7 animate-spin text-accent-ink" />
          <div className="text-sm font-medium text-fg">Generating OAuth authorization URL...</div>
        </div>
      ) : step === "pkce" && session ? (
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <Badge variant="info">
              {["antigravity", "agy", "gemini-cli"].includes(request.providerId)
                ? "Google OAuth 2.0"
                : "OAuth 2.0 PKCE"}
            </Badge>
            <span className="font-mono text-[11px] text-fg-muted">
              {session.provider} · {session.connection_id}
            </span>
          </div>

          {isLanLoopback ? (
            <div className="rounded-xl border border-accent/35 bg-accent-subtle p-3.5 text-xs leading-relaxed text-fg">
              <div className="mb-1 font-semibold text-accent-ink">
                Remote / LAN Server Mode ({window.location.hostname})
              </div>
              <ol className="list-decimal space-y-1 pl-4 text-fg/90">
                <li>Sign in and approve on the opened <strong>{request.displayName}</strong> page.</li>
                <li>
                  Your browser will redirect to <code className="rounded bg-surface px-1 py-0.5 font-mono text-[11px]">{session.redirect_uri}?code=...</code> (an unreachable localhost screen is expected on LAN).
                </li>
                <li>
                  <strong>Copy the full URL</strong> from that browser address bar and paste it below.
                </li>
              </ol>
            </div>
          ) : request.providerId === "claude" ? (
            <div className="rounded-xl border border-accent/35 bg-accent-subtle p-3.5 text-xs leading-relaxed text-fg">
              Complete login on <strong>Claude</strong>, then copy the displayed authentication code (<code className="font-mono">code#state</code>) and paste it below.
            </div>
          ) : (
            <div className="flex items-center gap-2.5 rounded-xl border border-success/30 bg-success-bg p-3 text-xs text-success">
              <Loader2 className="h-4 w-4 shrink-0 animate-spin" />
              <span>Waiting for popup callback... or paste the full callback URL manually below.</span>
            </div>
          )}

          <div className="flex flex-wrap gap-2">
            <a
              href={authUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex h-9 items-center gap-2 rounded-lg bg-accent px-3.5 text-xs font-semibold text-accent-fg hover:bg-accent-hover"
            >
              <ExternalLink className="h-3.5 w-3.5" />
              Open {request.displayName} Login
            </a>
            <Button variant="default" size="sm" onClick={() => copy(authUrl, "Login URL copied")}>
              <Copy className="h-3.5 w-3.5" />
              Copy Login URL
            </Button>
          </div>

          <div className="space-y-1.5">
            <label className="text-xs font-medium text-fg">
              Paste full callback URL or authorization code
            </label>
            <Input
              autoFocus={isLanLoopback || request.providerId === "claude"}
              value={cbInput}
              onChange={(e) => setCbInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void completeCallback(session.provider, session.state, cbInput);
              }}
              placeholder={`${session.redirect_uri}?code=...&state=...`}
            />
          </div>

          <div className="flex justify-end gap-2 pt-1">
            <Button variant="ghost" onClick={onClose}>Cancel</Button>
            <Button
              variant="primary"
              disabled={submitting || !cbInput.trim()}
              onClick={() => void completeCallback(session.provider, session.state, cbInput)}
            >
              {submitting ? (
                <>
                  <Loader2 className="h-3.5 w-3.5 animate-spin" /> Exchanging...
                </>
              ) : (
                "Complete Connection"
              )}
            </Button>
          </div>
        </div>
      ) : step === "device" && session ? (
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <Badge variant="accent">RFC 8628 Device Code</Badge>
            <Badge variant="default" className="font-mono">
              Expires {Math.floor(remainingSec / 60)}m {remainingSec % 60}s
            </Badge>
          </div>

          <div className="rounded-xl border border-border-strong bg-surface-subtle p-5 text-center">
            <div className="mb-1.5 text-xs text-fg-muted">Enter this code on the verification page</div>
            <div className="font-mono text-3xl font-bold tracking-[0.18em] text-accent-ink">
              {session.user_code}
            </div>
          </div>

          <div className="flex flex-wrap gap-2">
            <a
              href={verifyUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex h-9 items-center gap-2 rounded-lg bg-accent px-3.5 text-xs font-semibold text-accent-fg hover:bg-accent-hover"
            >
              <ExternalLink className="h-3.5 w-3.5" />
              Open Verification Page
            </a>
            <Button
              variant="default"
              size="sm"
              onClick={() => copy(session.user_code, "Device code copied")}
            >
              <Copy className="h-3.5 w-3.5" />
              Copy Code
            </Button>
          </div>

          <div className="flex items-center gap-2 text-xs text-fg-muted">
            <Loader2 className="h-3.5 w-3.5 animate-spin text-accent-ink" />
            <span>Auto-polling every {Math.max(3, Number(session.interval) || 5)}s until approved...</span>
          </div>
        </div>
      ) : step === "import_only" && session ? (
        <div className="space-y-4">
          <div className="rounded-xl border border-border bg-surface-subtle p-4 text-xs leading-relaxed text-fg-muted">
            <strong className="text-fg">{request.displayName}</strong> authenticates by importing an existing IDE session token. Sign in on the provider website, then switch to <strong>Paste Token</strong> above.
          </div>
          <div className="flex gap-2">
            {authUrl && (
              <a
                href={authUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex h-9 items-center gap-2 rounded-lg bg-accent px-3.5 text-xs font-semibold text-accent-fg hover:bg-accent-hover"
              >
                <ExternalLink className="h-3.5 w-3.5" />
                Open {request.displayName}
              </a>
            )}
            <Button variant="default" size="sm" onClick={() => setTab("paste")}>
              Switch to Paste Token
            </Button>
          </div>
        </div>
      ) : step === "success" ? (
        <div className="flex flex-col items-center py-6 text-center">
          <div className="mb-3 flex h-14 w-14 items-center justify-center rounded-full bg-success-bg text-success">
            <CheckCircle2 className="h-7 w-7" />
          </div>
          <h3 className="text-base font-semibold text-fg">Connected {request.displayName}</h3>
          <p className="mt-1 font-mono text-xs text-fg-muted">
            {successInfo?.email || successInfo?.label || successInfo?.connection_id}
          </p>
          {successInfo?.project_id && (
            <Badge variant="success" className="mt-2 font-mono">
              Cloud Code Project: {successInfo.project_id}
            </Badge>
          )}
          <Button variant="primary" className="mt-5 w-full" onClick={onClose}>
            Done
          </Button>
        </div>
      ) : (
        <div className="space-y-4">
          <div className="flex items-start gap-3 rounded-xl border border-danger/35 bg-danger-bg p-3.5 text-xs text-danger">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <div className="break-words">{errorMsg || "Authentication failed"}</div>
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={onClose}>Close</Button>
            <Button
              variant="primary"
              onClick={() => {
                setStep("loading");
                setErrorMsg("");
              }}
              className="hidden"
            >
              <RefreshCw className="h-3.5 w-3.5" /> Retry
            </Button>
          </div>
        </div>
      )}
    </Dialog>
  );
}
