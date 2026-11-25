import { CheckCircle2, AlertCircle, Info, X } from "lucide-react";
import { cn } from "@/lib/utils";

export interface ToastItem {
  id: number;
  message: string;
  tone?: "success" | "error" | "info";
}

export function ToastStack({
  toasts,
  onDismiss,
}: {
  toasts: ToastItem[];
  onDismiss: (id: number) => void;
}) {
  if (!toasts.length) return null;
  return (
    <div className="fixed right-5 bottom-5 z-[60] flex max-w-sm flex-col gap-2">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={cn(
            "flex items-center gap-2.5 rounded-xl border px-3.5 py-2.5 text-sm shadow-xl backdrop-blur-md transition-all",
            t.tone === "error"
              ? "border-danger/40 bg-danger-bg text-danger"
              : t.tone === "info"
                ? "border-border-strong bg-surface text-fg"
                : "border-success/35 bg-surface text-fg"
          )}
        >
          {t.tone === "error" ? (
            <AlertCircle className="h-4 w-4 shrink-0 text-danger" />
          ) : t.tone === "info" ? (
            <Info className="h-4 w-4 shrink-0 text-accent-ink" />
          ) : (
            <CheckCircle2 className="h-4 w-4 shrink-0 text-success" />
          )}
          <span className="flex-1 text-xs font-medium leading-snug">{t.message}</span>
          <button
            type="button"
            onClick={() => onDismiss(t.id)}
            className="text-fg-muted hover:text-fg cursor-pointer"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
    </div>
  );
}
