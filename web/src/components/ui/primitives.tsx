import React, { useEffect } from "react";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";

// ── Button ─────────────────────────────────────────────────────────

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "default" | "primary" | "secondary" | "ghost" | "danger" | "outline";
  size?: "sm" | "md" | "lg" | "icon";
}

export function Button({
  variant = "default",
  size = "md",
  className,
  ...props
}: ButtonProps) {
  return (
    <button
      {...props}
      className={cn(
        "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-lg font-medium transition-all duration-150 cursor-pointer disabled:pointer-events-none disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        variant === "primary" &&
          "bg-accent text-accent-fg hover:bg-accent-hover shadow-xs font-semibold",
        variant === "default" &&
          "bg-surface-subtle text-fg border border-border hover:bg-surface-muted hover:border-border-strong",
        variant === "outline" &&
          "bg-transparent text-fg border border-border-strong hover:bg-surface-subtle",
        variant === "secondary" &&
          "bg-surface-muted text-fg hover:bg-surface-subtle",
        variant === "ghost" &&
          "bg-transparent text-fg-muted hover:text-fg hover:bg-surface-subtle",
        variant === "danger" &&
          "bg-danger-bg text-danger border border-danger/30 hover:bg-danger/20",
        size === "sm" && "h-8 px-3 text-xs rounded-md",
        size === "md" && "h-9 px-3.5 text-sm",
        size === "lg" && "h-10 px-5 text-sm",
        size === "icon" && "h-8 w-8 p-0 rounded-md",
        className
      )}
    />
  );
}

// ── Card ───────────────────────────────────────────────────────────

export function Card({
  className,
  children,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      {...props}
      className={cn(
        "rounded-xl border border-border bg-surface text-fg transition-colors",
        className
      )}
    >
      {children}
    </div>
  );
}

export function CardHeader({
  className,
  children,
}: {
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div className={cn("flex items-center justify-between gap-3 px-5 pt-4 pb-3", className)}>
      {children}
    </div>
  );
}

export function CardTitle({
  className,
  children,
}: {
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <h3 className={cn("text-sm font-semibold tracking-tight text-fg", className)}>
      {children}
    </h3>
  );
}

export function CardDescription({
  className,
  children,
}: {
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <p className={cn("mt-0.5 text-xs text-fg-muted", className)}>{children}</p>
  );
}

export function StatCard({
  label,
  value,
  sub,
  icon,
}: {
  label: string;
  value: React.ReactNode;
  sub?: React.ReactNode;
  icon?: React.ReactNode;
}) {
  return (
    <div className="rounded-xl border border-border bg-surface p-4">
      <div className="flex items-center justify-between gap-2">
        <span className="text-[11px] font-medium uppercase tracking-wider text-fg-muted">
          {label}
        </span>
        {icon}
      </div>
      <div className="mt-1.5 font-mono text-xl font-semibold tracking-tight text-fg">
        {value}
      </div>
      {sub && <div className="mt-1 text-[11px] text-fg-muted">{sub}</div>}
    </div>
  );
}

export function CardContent({
  className,
  children,
}: {
  className?: string;
  children: React.ReactNode;
}) {
  return <div className={cn("px-5 pb-5", className)}>{children}</div>;
}

// ── Badge ──────────────────────────────────────────────────────────

export function Badge({
  variant = "default",
  className,
  children,
}: {
  variant?: "default" | "accent" | "success" | "warning" | "danger" | "info" | "outline";
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-[11px] font-medium leading-4 whitespace-nowrap",
        variant === "default" && "bg-surface-subtle text-fg-muted border border-border",
        variant === "outline" && "border border-border-strong text-fg-muted",
        variant === "accent" && "bg-accent-subtle text-accent-ink border border-accent/30",
        variant === "success" && "bg-success-bg text-success border border-success/25",
        variant === "warning" && "bg-warning-bg text-warning border border-warning/25",
        variant === "danger" && "bg-danger-bg text-danger border border-danger/25",
        variant === "info" && "bg-info-bg text-info border border-info/25",
        className
      )}
    >
      {children}
    </span>
  );
}

// ── Input & Select ─────────────────────────────────────────────────

export function Input({
  className,
  ...props
}: React.InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      {...props}
      className={cn(
        "h-9 w-full rounded-lg border border-border-strong bg-surface px-3 text-sm text-fg placeholder:text-fg-muted/70 transition-colors focus:border-ring focus:outline-none focus:ring-2 focus:ring-ring/20 disabled:opacity-50",
        className
      )}
    />
  );
}

export function Select({
  className,
  ...props
}: React.SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select
      {...props}
      className={cn(
        "h-9 rounded-lg border border-border-strong bg-surface px-3 text-sm text-fg transition-colors focus:border-ring focus:outline-none focus:ring-2 focus:ring-ring/20",
        className
      )}
    />
  );
}

// ── Switch ─────────────────────────────────────────────────────────

export function Switch({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full border transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50",
        checked ? "bg-accent border-accent" : "bg-surface-muted border-border-strong"
      )}
    >
      <span
        className={cn(
          "pointer-events-none block h-4 w-4 rounded-full shadow-xs transition-transform duration-150",
          checked ? "translate-x-5.5 bg-accent-fg" : "translate-x-1 bg-fg-muted"
        )}
      />
    </button>
  );
}

// ── Segmented Control ──────────────────────────────────────────────

export function SegmentedControl<T extends string>({
  value,
  options,
  onChange,
  className,
}: {
  value: T;
  options: Array<{ value: T; label: string }>;
  onChange: (v: T) => void;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "inline-flex items-center rounded-lg border border-border bg-surface-subtle p-0.5",
        className
      )}
    >
      {options.map((opt) => (
        <button
          key={opt.value}
          type="button"
          onClick={() => onChange(opt.value)}
          className={cn(
            "rounded-md px-3 py-1 text-xs font-medium transition-all cursor-pointer",
            value === opt.value
              ? "bg-surface text-fg shadow-xs border border-border"
              : "text-fg-muted hover:text-fg border border-transparent"
          )}
        >
          {opt.label}
        </button>
      ))}
    </div>
  );
}

// ── Modal / Dialog ─────────────────────────────────────────────────

export function Dialog({
  open,
  onClose,
  title,
  description,
  icon,
  maxWidth = "max-w-lg",
  children,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: string;
  icon?: React.ReactNode;
  maxWidth?: string;
  children: React.ReactNode;
}) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-[2px] p-4 animate-in fade-in duration-150"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        className={cn(
          "w-full rounded-2xl border border-border-strong bg-surface p-6 shadow-2xl",
          maxWidth
        )}
      >
        <div className="mb-4 flex items-start justify-between gap-4">
          <div className="flex items-center gap-3 min-w-0">
            {icon && (
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-border bg-surface-subtle">
                {icon}
              </div>
            )}
            <div className="min-w-0">
              <h2 className="truncate text-base font-semibold tracking-tight text-fg">
                {title}
              </h2>
              {description && (
                <p className="mt-0.5 text-xs text-fg-muted">{description}</p>
              )}
            </div>
          </div>
          <Button variant="ghost" size="icon" onClick={onClose} aria-label="Close">
            <X className="h-4 w-4" />
          </Button>
        </div>
        {children}
      </div>
    </div>
  );
}

// ── Empty State ────────────────────────────────────────────────────

export function EmptyState({
  icon,
  title,
  description,
  action,
  className,
}: {
  icon?: React.ReactNode;
  title: string;
  description?: string;
  action?: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center rounded-xl border border-dashed border-border-strong bg-surface-subtle/50 px-6 py-10 text-center",
        className
      )}
    >
      {icon && (
        <div className="mb-3 flex h-11 w-11 items-center justify-center rounded-xl border border-border bg-surface text-fg-muted">
          {icon}
        </div>
      )}
      <div className="text-sm font-semibold text-fg">{title}</div>
      {description && (
        <p className="mt-1 max-w-md text-xs leading-relaxed text-fg-muted">
          {description}
        </p>
      )}
      {action && <div className="mt-4">{action}</div>}
    </div>
  );
}

// ── Page Header ────────────────────────────────────────────────────

export function PageHeader({
  title,
  description,
  badge,
  actions,
}: {
  title: string;
  description?: string;
  badge?: React.ReactNode;
  actions?: React.ReactNode;
}) {
  return (
    <div className="mb-6 flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
      <div>
        <div className="flex items-center gap-2.5">
          <h1 className="text-2xl font-semibold tracking-tight text-fg">{title}</h1>
          {badge}
        </div>
        {description && (
          <p className="mt-1 text-sm text-fg-muted">{description}</p>
        )}
      </div>
      {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
    </div>
  );
}
