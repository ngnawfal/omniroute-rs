import type { ComponentType, CSSProperties } from "react";

import Ai21MonoIcon from "@lobehub/icons/es/Ai21/components/Mono";
import AntigravityColorIcon from "@lobehub/icons/es/Antigravity/components/Color";
import BedrockColorIcon from "@lobehub/icons/es/Bedrock/components/Color";
import BflMonoIcon from "@lobehub/icons/es/Bfl/components/Mono";
import ClaudeCodeColorIcon from "@lobehub/icons/es/ClaudeCode/components/Color";
import CodexColorIcon from "@lobehub/icons/es/Codex/components/Color";
import DbrxColorIcon from "@lobehub/icons/es/Dbrx/components/Color";
import DevinColorIcon from "@lobehub/icons/es/Devin/components/Color";
import FeatherlessColorIcon from "@lobehub/icons/es/Featherless/components/Color";
import GithubMonoIcon from "@lobehub/icons/es/Github/components/Mono";
import GithubCopilotMonoIcon from "@lobehub/icons/es/GithubCopilot/components/Mono";
import JinaMonoIcon from "@lobehub/icons/es/Jina/components/Mono";
import KiloCodeMonoIcon from "@lobehub/icons/es/KiloCode/components/Mono";
import LmStudioMonoIcon from "@lobehub/icons/es/LmStudio/components/Mono";
import MetaAIColorIcon from "@lobehub/icons/es/MetaAI/components/Color";
import NanoBananaColorIcon from "@lobehub/icons/es/NanoBanana/components/Color";
import NousResearchMonoIcon from "@lobehub/icons/es/NousResearch/components/Mono";
import OpenCodeMonoIcon from "@lobehub/icons/es/OpenCode/components/Mono";
import QoderColorIcon from "@lobehub/icons/es/Qoder/components/Color";
import SiliconCloudColorIcon from "@lobehub/icons/es/SiliconCloud/components/Color";
import StabilityColorIcon from "@lobehub/icons/es/Stability/components/Color";
import TogetherColorIcon from "@lobehub/icons/es/Together/components/Color";
import VertexAIColorIcon from "@lobehub/icons/es/VertexAI/components/Color";

export type BrandSvgIcon = ComponentType<{ size?: number; style?: CSSProperties; className?: string }>;

export const LOBE_BRAND_ICONS: Record<string, BrandSvgIcon> = {
  antigravity: AntigravityColorIcon,
  agy: AntigravityColorIcon,
  bedrock: BedrockColorIcon,
  "aws-bedrock": BedrockColorIcon,
  vertex: VertexAIColorIcon,
  vertexai: VertexAIColorIcon,
  "vertex-ai": VertexAIColorIcon,
  "vertex-partner": VertexAIColorIcon,
  "google-vertex": VertexAIColorIcon,
  devin: DevinColorIcon,
  "devin-desktop": DevinColorIcon,
  "devin-cli": DevinColorIcon,
  github: GithubCopilotMonoIcon,
  "github-copilot": GithubCopilotMonoIcon,
  "ghe-copilot": GithubCopilotMonoIcon,
  "github-models": GithubMonoIcon,
  "claude-code": ClaudeCodeColorIcon,
  codex: CodexColorIcon,
  together: TogetherColorIcon,
  "together-ai": TogetherColorIcon,
  togetherai: TogetherColorIcon,
  siliconcloud: SiliconCloudColorIcon,
  siliconflow: SiliconCloudColorIcon,
  stability: StabilityColorIcon,
  lmstudio: LmStudioMonoIcon,
  "lm-studio": LmStudioMonoIcon,
  nousresearch: NousResearchMonoIcon,
  qoder: QoderColorIcon,
  kilocode: KiloCodeMonoIcon,
  "kilo-gateway": KiloCodeMonoIcon,
  jina: JinaMonoIcon,
  featherless: FeatherlessColorIcon,
  bfl: BflMonoIcon,
  ai21: Ai21MonoIcon,
  dbrx: DbrxColorIcon,
  databricks: DbrxColorIcon,
  nanobanana: NanoBananaColorIcon,
  "muse-code": MetaAIColorIcon,
  "muse-spark-web": MetaAIColorIcon,
  opencode: OpenCodeMonoIcon,
  "opencode-go": OpenCodeMonoIcon,
  "opencode-zen": OpenCodeMonoIcon,
};
