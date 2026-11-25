import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function fmtUptime(s: number): string {
  if (!Number.isFinite(s) || s < 0) return "0s";
  if (s < 60) return `${Math.floor(s)}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${Math.floor(s % 60)}s`;
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (h < 24) return `${h}h ${m}m`;
  const d = Math.floor(h / 24);
  return `${d}d ${h % 24}h`;
}

export function fmtNumber(n: number | undefined | null): string {
  return Number(n ?? 0).toLocaleString();
}

export const fmtNum = fmtNumber;

export function fmtCost(usd: number | undefined | null): string {
  return `$${Number(usd ?? 0).toFixed(4)}`;
}

export function fmtRelativeExpiry(ms?: number | null): string {
  if (!ms) return "No expiry";
  const diff = ms - Date.now();
  if (diff <= 0) return "Expired";
  const mins = Math.floor(diff / 60_000);
  if (mins < 60) return `in ${mins}m`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 48) return `in ${hrs}h`;
  return `in ${Math.floor(hrs / 24)}d`;
}

/**
 * Reliable clipboard copy that works on both HTTPS/localhost and plain HTTP LAN IPs
 * (e.g. http://192.168.11.209:20128) where navigator.clipboard is blocked by browsers.
 */
export async function copyToClipboard(text: string): Promise<boolean> {
  try {
    if (window.isSecureContext && navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // Fall through to legacy textarea copy
  }
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.top = "-9999px";
    ta.style.left = "-9999px";
    document.body.appendChild(ta);
    ta.focus();
    ta.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}
