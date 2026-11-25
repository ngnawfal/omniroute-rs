import { createElement, memo, useState } from "react";
import { cn } from "@/lib/utils";
import { LOBE_BRAND_ICONS } from "./lobeBrandIcons";

// All 146 official SVG assets copied directly from upstream OmniRoute `public/providers/`
const KNOWN_SVGS = new Set([
  "360ai", "alibaba", "anthropic", "arcee", "arcee-ai", "assemblyai", "auggie", "aws",
  "azure", "azureai", "baichuan", "baidu", "bailian", "baseten", "bluesminds", "bytez",
  "cerebras", "charm-hyper", "chutes", "claude", "claude-web", "cline", "cloudflare",
  "codex", "cohere", "comfyui", "continue", "copilot", "coze", "crof", "cursor",
  "deepgram", "deepinfra", "deepseek", "dgrid", "dify", "dit", "doubao", "elevenlabs",
  "exa", "factory", "fal", "fireworks", "freeaiapikey", "freemodel-dev", "friendli",
  "galadriel", "gemini", "gitlawb", "gitlawb-gmi", "google", "grok", "groq", "hcnsec",
  "heroku", "huggingchat", "huggingface", "hyperbolic", "ibm", "iflytek", "inclusionai",
  "inference", "kenari", "kimi", "kiro", "krutrim", "lambda", "liquid", "llm7",
  "longcat", "meta", "metaai", "minimax", "mistral", "monsterapi", "moonshot", "morph",
  "nebius", "nlpcloud", "nomic", "notrack-web", "novita", "nube", "nvidia", "ollama",
  "openadapter", "openai", "openclaw", "openrouter", "opper", "orcarouter", "ovhcloud",
  "perplexity", "phind", "picoclaw", "pioneer", "poe", "pollinations", "poolside",
  "publicai", "qwen", "recraft", "replicate", "requesty", "roocode", "runcrate", "runway",
  "sambanova", "searchapi", "sensenova", "snowflake", "sparkdesk", "stepfun", "sumopod",
  "t3-web", "tavily", "tencent", "tokenrouter", "topazlabs", "trae", "udio", "uncloseai",
  "upstage", "v0", "veoaifree-web", "vercel", "vllm", "volcengine", "voyage", "wafer",
  "x5lab", "xai", "xinference", "yi", "yuanbao-web", "zed-hosted", "zenmux",
  "zenmux-free", "zhipu",
]);

// Monochrome white SVGs (invert in Light mode so they stay crisp black on light paper)
const WHITE_MONO_SVGS = new Set([
  "anthropic", "grok", "ollama", "openai", "openrouter", "qwen", "replicate", "v0", "vercel", "windsurf",
]);

// Monochrome dark/currentColor SVGs (invert in Dark mode so they stay crisp white on dark obsidian)
const DARK_MONO_SVGS = new Set([
  "baseten", "cline", "elevenlabs", "friendli", "inference", "lambda", "moonshot",
  "nebius", "pollinations", "recraft", "roocode", "runcrate", "searchapi", "suno",
  "topazlabs", "upstage", "xai",
]);

const ALIASES: Record<string, string> = {
  "gemini-cli": "gemini",
  "google-ai": "google",
  "google-genai": "google",
  "grok-cli": "grok",
  "xai-oauth": "xai",
  "minimax-code": "minimax",
  "amazon-q": "kiro",
  "azure-openai": "azure",
  "azure-foundry": "azureai",
  "cursor-api": "cursor",
  clinepass: "cline",
  bigmodel: "zhipu",
  glm: "zhipu",
  "openai-native": "openai",
  "workers-ai": "cloudflare",
  "cloudflare-workers-ai": "cloudflare",
  "fireworks-ai": "fireworks",
  "mistral-ai": "mistral",
  "hugging-face": "huggingface",
  "perplexity-ai": "perplexity",
  "nvidia-nim": "nvidia",
  "samba-nova": "sambanova",
  "deep-infra": "deepinfra",
  "alibaba-cloud": "alibaba",
  dashscope: "alibaba",
};

const THEMED_SVGS: Record<string, { light: string; dark: string }> = {
  chatgpt: {
    light: "/providers/chatgpt-logo-black.svg",
    dark: "/providers/chatgpt-logo-white.svg",
  },
  "kimi-coding": {
    light: "/providers/kimi-logomark-light.svg",
    dark: "/providers/kimi-logomark-dark.svg",
  },
  moonshot: {
    light: "/providers/kimi-logomark-light.svg",
    dark: "/providers/kimi-logomark-dark.svg",
  },
};

const MONOGRAM_COLORS = [
  "bg-amber-500/15 text-amber-500 border-amber-500/25",
  "bg-emerald-500/15 text-emerald-500 border-emerald-500/25",
  "bg-sky-500/15 text-sky-500 border-sky-500/25",
  "bg-violet-500/15 text-violet-400 border-violet-500/25",
  "bg-rose-500/15 text-rose-400 border-rose-500/25",
  "bg-cyan-500/15 text-cyan-400 border-cyan-500/25",
];

function hashCode(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return Math.abs(h);
}

export const ProviderIcon = memo(function ProviderIcon({
  providerId,
  size = 24,
  isDark = true,
  className,
}: {
  providerId: string;
  size?: number;
  isDark?: boolean;
  className?: string;
}) {
  const [localFailed, setLocalFailed] = useState(false);
  const [cdnFailed, setCdnFailed] = useState(false);
  const norm = (providerId || "").toLowerCase().trim();

  // Tier 1: Official @lobehub/icons color/brand SVGs (Antigravity, Bedrock, Vertex AI, Devin, Together, GitHub Copilot, etc.)
  const lobeIcon = LOBE_BRAND_ICONS[norm];
  if (lobeIcon) {
    return (
      <span
        className={cn("inline-flex shrink-0 items-center justify-center text-fg", className)}
        style={{ width: size, height: size }}
      >
        {createElement(lobeIcon, { size })}
      </span>
    );
  }

  // Tier 2: Theme-aware official local SVGs (ChatGPT, Kimi)
  if (!localFailed && THEMED_SVGS[norm]) {
    const src = isDark ? THEMED_SVGS[norm].dark : THEMED_SVGS[norm].light;
    return (
      <img
        src={src}
        alt={providerId}
        width={size}
        height={size}
        style={{ width: size, height: size }}
        className={cn("shrink-0 object-contain rounded-sm", className)}
        onError={() => setLocalFailed(true)}
      />
    );
  }

  // Tier 3: 146 official upstream local SVGs embedded in binary (/providers/{slug}.svg)
  const mapped = ALIASES[norm] || norm;
  const baseSlug = KNOWN_SVGS.has(mapped)
    ? mapped
    : KNOWN_SVGS.has(mapped.split("-")[0])
      ? mapped.split("-")[0]
      : null;

  if (!localFailed && baseSlug) {
    const needsInvert =
      (WHITE_MONO_SVGS.has(baseSlug) && !isDark) ||
      (DARK_MONO_SVGS.has(baseSlug) && isDark);
    return (
      <img
        src={`/providers/${baseSlug}.svg`}
        alt={providerId}
        width={size}
        height={size}
        style={{ width: size, height: size }}
        className={cn("shrink-0 object-contain", needsInvert && "invert", className)}
        onError={() => setLocalFailed(true)}
      />
    );
  }

  // Tier 4: External vector SVG fallback via thesvg.org (identical to upstream ProviderIcon.tsx Tier 5)
  const cdnSlug = (mapped.replace(/[^a-z0-9-]/g, "") || norm).split("-")[0];
  if (!cdnFailed && cdnSlug) {
    return (
      <img
        src={`https://thesvg.org/icons/${cdnSlug}/default.svg`}
        alt={providerId}
        width={size}
        height={size}
        style={{ width: size, height: size }}
        className={cn("shrink-0 object-contain", className)}
        onError={() => setCdnFailed(true)}
      />
    );
  }

  // Tier 5: Crisp fallback monogram badge only when no vector mark exists anywhere
  const tone = MONOGRAM_COLORS[hashCode(norm) % MONOGRAM_COLORS.length];
  const letters = (norm.replace(/[^a-z0-9]/g, "").slice(0, 2) || "AI").toUpperCase();
  return (
    <span
      style={{ width: size, height: size, fontSize: Math.max(9, Math.round(size * 0.4)) }}
      className={cn(
        "inline-flex shrink-0 items-center justify-center rounded-md border font-mono font-bold leading-none select-none",
        tone,
        className
      )}
    >
      {letters}
    </span>
  );
});
