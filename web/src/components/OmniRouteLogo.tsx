import { cn } from "@/lib/utils";

export function OmniRouteLogo({
  size = 28,
  className,
}: {
  size?: number;
  className?: string;
}) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 32 32"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={cn("shrink-0", className)}
      aria-hidden="true"
    >
      <rect width="32" height="32" rx="8" fill="var(--accent)" />
      <circle cx="16" cy="16" r="3" fill="var(--accent-fg)" />
      <circle cx="8.5" cy="8.5" r="2" fill="var(--accent-fg)" />
      <circle cx="23.5" cy="8.5" r="2" fill="var(--accent-fg)" />
      <circle cx="8.5" cy="23.5" r="2" fill="var(--accent-fg)" />
      <circle cx="23.5" cy="23.5" r="2" fill="var(--accent-fg)" />
      <circle cx="16" cy="5.5" r="1.5" fill="var(--accent-fg)" />
      <circle cx="16" cy="26.5" r="1.5" fill="var(--accent-fg)" />
      <line x1="16" y1="13" x2="8.5" y2="8.5" stroke="var(--accent-fg)" strokeWidth="1.4" strokeLinecap="round" />
      <line x1="16" y1="13" x2="23.5" y2="8.5" stroke="var(--accent-fg)" strokeWidth="1.4" strokeLinecap="round" />
      <line x1="16" y1="19" x2="8.5" y2="23.5" stroke="var(--accent-fg)" strokeWidth="1.4" strokeLinecap="round" />
      <line x1="16" y1="19" x2="23.5" y2="23.5" stroke="var(--accent-fg)" strokeWidth="1.4" strokeLinecap="round" />
      <line x1="16" y1="13" x2="16" y2="5.5" stroke="var(--accent-fg)" strokeWidth="1.4" strokeLinecap="round" />
      <line x1="16" y1="19" x2="16" y2="26.5" stroke="var(--accent-fg)" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  );
}
