import { useEffect, useRef, useState } from "react";
import { ArrowRight, Eye, EyeOff, Moon, Palette, Sun } from "lucide-react";
import { OmniRouteLogo } from "@/components/OmniRouteLogo";
import { cn } from "@/lib/utils";

interface LoginPageProps {
  theme: "dark" | "light";
  appearance: "editions" | "classic";
  onToggleTheme: () => void;
  onToggleAppearance: () => void;
  onLogin: (password: string) => Promise<{ ok: boolean; error?: string; retryAfter?: number }>;
}

export function LoginPage({
  theme,
  appearance,
  onToggleTheme,
  onToggleAppearance,
  onLogin,
}: LoginPageProps) {
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [capsLockOn, setCapsLockOn] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState("");
  const [lockoutSecs, setLockoutSecs] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const host =
    typeof window !== "undefined" && window.location.host
      ? window.location.host
      : "127.0.0.1:20128";

  useEffect(() => {
    if (lockoutSecs <= 0) return;
    const t = window.setInterval(() => {
      setLockoutSecs((s) => (s <= 1 ? 0 : s - 1));
    }, 1000);
    return () => window.clearInterval(t);
  }, [lockoutSecs]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!password.trim() || submitting || lockoutSecs > 0) return;
    setSubmitting(true);
    setError("");
    const res = await onLogin(password);
    setSubmitting(false);
    if (!res.ok) {
      setError(res.error || "Incorrect password.");
      if (res.retryAfter && res.retryAfter > 0) {
        setLockoutSecs(res.retryAfter);
      }
      inputRef.current?.select();
    } else {
      setPassword("");
    }
  };

  const handleKeyModifiers = (e: React.KeyboardEvent<HTMLInputElement>) => {
    setCapsLockOn(e.getModifierState("CapsLock"));
  };

  return (
    <div className="grid min-h-[100dvh] grid-cols-1 bg-bg text-fg lg:grid-cols-12">
      {/* Left Architectural Surface (Desktop) */}
      <aside
        className="relative hidden flex-col justify-between overflow-hidden p-10 lg:col-span-7 lg:flex xl:p-14"
        style={{
          backgroundImage:
            "linear-gradient(to right, var(--border) 1px, transparent 1px), linear-gradient(to bottom, var(--border) 1px, transparent 1px)",
          backgroundSize: "56px 56px",
          backgroundPosition: "-1px -1px",
        }}
      >
        {/* Subtle edge vignette so hairline grid fades gracefully into canvas */}
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-0"
          style={{
            background:
              "radial-gradient(ellipse 85% 75% at 18% 82%, transparent 0%, var(--bg) 78%)",
          }}
        />

        {/* Top bar */}
        <div className="relative z-10 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <OmniRouteLogo size={26} />
            <div className="flex items-baseline gap-2.5">
              <span className="text-[15px] font-semibold tracking-tight text-fg">
                OmniRoute
              </span>
              <span className="font-mono text-[11px] text-fg-muted">rs · v0.1.0</span>
            </div>
          </div>

          <div className="flex items-center gap-1.5">
            <button
              type="button"
              onClick={onToggleAppearance}
              title="Switch palette (Editions / Classic)"
              className="inline-flex h-8 items-center gap-1.5 rounded-md border border-border bg-bg/80 px-2.5 text-xs text-fg-muted backdrop-blur-xs transition-colors hover:border-border-strong hover:text-fg cursor-pointer"
            >
              <Palette className="h-3.5 w-3.5 text-accent-ink" strokeWidth={1.75} />
              <span className="capitalize">{appearance}</span>
            </button>
            <button
              type="button"
              onClick={onToggleTheme}
              title={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
              aria-label={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
              className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-border bg-bg/80 text-fg-muted backdrop-blur-xs transition-colors hover:border-border-strong hover:text-fg cursor-pointer"
            >
              {theme === "dark" ? (
                <Sun className="h-3.5 w-3.5" strokeWidth={1.75} />
              ) : (
                <Moon className="h-3.5 w-3.5" strokeWidth={1.75} />
              )}
            </button>
          </div>
        </div>

        {/* Lower Editorial Anchor */}
        <div className="relative z-10 max-w-lg space-y-10">
          <div className="space-y-4">
            <h1 className="text-4xl font-semibold tracking-[-0.035em] leading-[1.06] text-fg xl:text-[46px]">
              One gateway for
              <br />
              every model.
            </h1>
            <p className="max-w-[42ch] text-sm leading-relaxed text-fg-muted">
              Multi-provider failover, OAuth token lifecycle, prompt compression, and call
              telemetry running natively on your own hardware.
            </p>
          </div>

          {/* Hairline-separated live node specifications */}
          <dl className="grid grid-cols-3 border-t border-border pt-5">
            <div className="pr-4">
              <dt className="font-mono text-[10px] tracking-[0.14em] text-fg-muted uppercase">
                Node
              </dt>
              <dd className="mt-1.5 truncate font-mono text-xs text-fg" title={host}>
                {host}
              </dd>
            </div>
            <div className="border-l border-border px-4">
              <dt className="font-mono text-[10px] tracking-[0.14em] text-fg-muted uppercase">
                Engine
              </dt>
              <dd className="mt-1.5 font-mono text-xs text-fg">Rust · SQLite WAL</dd>
            </div>
            <div className="border-l border-border pl-4">
              <dt className="font-mono text-[10px] tracking-[0.14em] text-fg-muted uppercase">
                Status
              </dt>
              <dd className="mt-1.5 flex items-center gap-2 font-mono text-xs text-fg">
                <span className="h-1.5 w-1.5 rounded-full bg-success" />
                Online
              </dd>
            </div>
          </dl>
        </div>
      </aside>

      {/* Right Authentication Pane */}
      <main className="flex flex-col justify-between bg-surface lg:col-span-5 lg:border-l lg:border-border">
        {/* Compact mobile header */}
        <header className="flex h-14 items-center justify-between border-b border-border px-5 lg:hidden">
          <div className="flex items-center gap-2.5">
            <OmniRouteLogo size={24} />
            <span className="text-sm font-semibold tracking-tight text-fg">OmniRoute</span>
          </div>
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              onClick={onToggleAppearance}
              className="inline-flex h-8 items-center gap-1 rounded-md border border-border px-2 text-xs text-fg-muted hover:text-fg cursor-pointer"
            >
              <Palette className="h-3.5 w-3.5 text-accent-ink" strokeWidth={1.75} />
              <span className="capitalize">{appearance}</span>
            </button>
            <button
              type="button"
              onClick={onToggleTheme}
              aria-label="Toggle color mode"
              className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-border text-fg-muted hover:text-fg cursor-pointer"
            >
              {theme === "dark" ? (
                <Sun className="h-3.5 w-3.5" strokeWidth={1.75} />
              ) : (
                <Moon className="h-3.5 w-3.5" strokeWidth={1.75} />
              )}
            </button>
          </div>
        </header>

        {/* Form container — strictly left-aligned */}
        <div className="flex flex-1 items-center px-6 py-12 sm:px-10 lg:px-12 xl:px-16">
          <div className="mx-auto w-full max-w-[340px]">
            <div className="space-y-1.5">
              <h2 className="text-2xl font-semibold tracking-[-0.025em] text-fg">Sign in</h2>
              <p className="text-sm text-fg-muted">
                Admin console on{" "}
                <span className="font-mono text-[13px] text-fg">{host}</span>
              </p>
            </div>

            <form onSubmit={handleSubmit} className="mt-8 space-y-5" noValidate>
              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <label
                    htmlFor="omniroute-dashboard-password"
                    className="text-xs font-medium text-fg"
                  >
                    Password
                  </label>
                  {capsLockOn && (
                    <span className="font-mono text-[11px] text-warning">Caps Lock on</span>
                  )}
                </div>

                <div className="relative">
                  <input
                    ref={inputRef}
                    id="omniroute-dashboard-password"
                    name="password"
                    type={showPassword ? "text" : "password"}
                    autoFocus
                    autoComplete="current-password"
                    spellCheck={false}
                    disabled={submitting || lockoutSecs > 0}
                    value={password}
                    onChange={(e) => {
                      setPassword(e.target.value);
                      if (error) setError("");
                    }}
                    onKeyUp={handleKeyModifiers}
                    onKeyDown={handleKeyModifiers}
                    placeholder="Enter admin password"
                    aria-invalid={!!error}
                    className={cn(
                      "h-11 w-full rounded-lg border bg-bg px-3.5 pr-10 text-sm text-fg placeholder:text-fg-muted/70 transition-colors focus:outline-none focus:ring-2 focus:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-60",
                      showPassword ? "font-mono" : "font-sans",
                      error
                        ? "border-danger focus:border-danger"
                        : "border-border-strong focus:border-ring"
                    )}
                  />
                  <button
                    type="button"
                    tabIndex={-1}
                    onClick={() => setShowPassword((v) => !v)}
                    aria-label={showPassword ? "Hide password" : "Show password"}
                    className="absolute right-2 top-1/2 -translate-y-1/2 inline-flex h-7 w-7 items-center justify-center rounded-md text-fg-muted transition-colors hover:text-fg cursor-pointer"
                  >
                    {showPassword ? (
                      <EyeOff className="h-4 w-4" strokeWidth={1.75} />
                    ) : (
                      <Eye className="h-4 w-4" strokeWidth={1.75} />
                    )}
                  </button>
                </div>

                {error && (
                  <div
                    role="alert"
                    className="border-l-2 border-danger bg-danger-bg/70 px-3 py-2 text-xs leading-snug text-danger"
                  >
                    {lockoutSecs > 0
                      ? `Too many failed attempts. Try again in ${lockoutSecs}s.`
                      : error}
                  </div>
                )}
              </div>

              <button
                type="submit"
                disabled={submitting || !password.trim() || lockoutSecs > 0}
                className="inline-flex h-11 w-full items-center justify-center gap-2 rounded-lg bg-accent px-4 text-sm font-semibold text-accent-fg transition-all duration-150 hover:bg-accent-hover active:scale-[0.99] disabled:pointer-events-none disabled:opacity-45 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring cursor-pointer"
              >
                {submitting ? (
                  <span>Verifying…</span>
                ) : lockoutSecs > 0 ? (
                  <span className="font-mono">Locked ({lockoutSecs}s)</span>
                ) : (
                  <>
                    <span>Continue</span>
                    <ArrowRight className="h-4 w-4" strokeWidth={2} />
                  </>
                )}
              </button>
            </form>

            <div className="mt-8 border-t border-border pt-5 text-xs leading-relaxed text-fg-muted">
              Inference routes on{" "}
              <code className="rounded bg-surface-subtle px-1 py-0.5 font-mono text-[11px] text-fg">
                /v1/*
              </code>{" "}
              stay online for valid Bearer API keys regardless of console lock state.
            </div>
          </div>
        </div>

        {/* Quiet bottom status bar */}
        <footer className="flex h-12 items-center justify-between border-t border-border px-6 text-[11px] text-fg-muted sm:px-10 lg:px-12">
          <span className="font-mono">HttpOnly session</span>
          <span className="font-mono">OmniRoute RS</span>
        </footer>
      </main>
    </div>
  );
}
