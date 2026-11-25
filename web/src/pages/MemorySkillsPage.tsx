import { useState } from "react";
import { BrainCircuit, Sparkles, Search, Trash2, Plus } from "lucide-react";
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
import { apiDelete, apiPost } from "@/lib/api";

export function MemorySkillsPage({
  memories,
  skills,
  onRefresh,
  onToast,
}: {
  memories: any[];
  skills: any[];
  onRefresh: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [mKey, setMKey] = useState("");
  const [mType, setMType] = useState("factual");
  const [mContent, setMContent] = useState("");
  const [searchQ, setSearchQ] = useState("");
  const [searchHits, setSearchHits] = useState<any[] | null>(null);

  const [skName, setSkName] = useState("");
  const [skDesc, setSkDesc] = useState("");
  const [skInst, setSkInst] = useState("");

  const addMemory = async () => {
    if (!mKey.trim() || !mContent.trim()) {
      onToast("Key and content are required", "error");
      return;
    }
    const r = await apiPost("/api/memory", {
      key: mKey.trim(),
      memory_type: mType,
      content: mContent.trim(),
    });
    if (r.ok) {
      setMKey("");
      setMContent("");
      setSearchHits(null);
      onToast("Memory stored", "success");
      onRefresh();
    } else onToast(r.error || "Failed to store memory", "error");
  };

  const searchMemory = async () => {
    if (!searchQ.trim()) {
      setSearchHits(null);
      return;
    }
    const r = await apiPost("/api/memory/retrieve", { query: searchQ.trim(), limit: 10 });
    if (r.ok) setSearchHits(r.data.memories || []);
  };

  const addSkill = async () => {
    if (!skName.trim() || !skInst.trim()) {
      onToast("Skill name and instructions are required", "error");
      return;
    }
    const r = await apiPost("/api/skills", {
      name: skName.trim(),
      description: skDesc.trim(),
      instructions: skInst.trim(),
    });
    if (r.ok) {
      setSkName("");
      setSkDesc("");
      setSkInst("");
      onToast("Skill installed", "success");
      onRefresh();
    } else onToast(r.error || "Failed to install skill", "error");
  };

  const visibleMemories = searchHits ?? memories;

  return (
    <div className="space-y-6">
      <PageHeader
        title="Persistent Memory & Skills"
        description="SQLite WAL agent memory with sparse TF-cosine retrieval and reusable agent skill templates."
      />

      <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
        {/* Agent Memories */}
        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <BrainCircuit className="h-4 w-4 text-accent-ink" />
              <CardTitle>Agent Memories</CardTitle>
            </div>
            <Badge variant="default">{memories.length} stored</Badge>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex flex-wrap gap-2">
              <Input
                value={mKey}
                onChange={(e) => setMKey(e.target.value)}
                placeholder="Key (e.g. deploy-host)"
                className="w-36"
              />
              <Select value={mType} onChange={(e) => setMType(e.target.value)} className="w-28 text-xs">
                <option value="factual">factual</option>
                <option value="episodic">episodic</option>
                <option value="procedural">procedural</option>
                <option value="semantic">semantic</option>
              </Select>
              <Input
                value={mContent}
                onChange={(e) => setMContent(e.target.value)}
                placeholder="Memory content..."
                className="flex-1"
              />
              <Button variant="primary" size="sm" onClick={addMemory}>
                <Plus className="h-3.5 w-3.5" /> Store
              </Button>
            </div>

            <div className="flex gap-2">
              <div className="relative flex-1">
                <Search className="pointer-events-none absolute top-1/2 left-3 h-3.5 w-3.5 -translate-y-1/2 text-fg-muted" />
                <Input
                  value={searchQ}
                  onChange={(e) => setSearchQ(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void searchMemory();
                  }}
                  placeholder="TF-cosine semantic search..."
                  className="pl-9 text-xs"
                />
              </div>
              <Button variant="default" size="sm" onClick={searchMemory}>
                Search
              </Button>
              {searchHits && (
                <Button variant="ghost" size="sm" onClick={() => { setSearchHits(null); setSearchQ(""); }}>
                  All
                </Button>
              )}
            </div>

            {visibleMemories.length === 0 ? (
              <EmptyState title="No memories stored" description="Store facts or preferences to retrieve them semantically." />
            ) : (
              <div className="space-y-2 max-h-80 overflow-y-auto">
                {visibleMemories.map((m: any) => (
                  <div
                    key={m.id}
                    className="flex items-start justify-between gap-3 rounded-xl border border-border bg-surface-subtle/40 p-3 text-xs"
                  >
                    <div className="min-w-0 flex-1 space-y-1">
                      <div className="flex items-center gap-2">
                        <span className="font-mono font-semibold text-fg">{m.key}</span>
                        <Badge variant="info">{m.memory_type}</Badge>
                        {typeof m.score === "number" && (
                          <Badge variant="accent" className="font-mono">
                            score {m.score.toFixed(2)}
                          </Badge>
                        )}
                      </div>
                      <p className="text-fg-muted">{m.content}</p>
                    </div>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="text-danger hover:bg-danger-bg"
                      onClick={async () => {
                        await apiDelete(`/api/memory/${encodeURIComponent(m.id)}`);
                        setSearchHits(null);
                        onToast("Memory deleted", "info");
                        onRefresh();
                      }}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>

        {/* Skills Registry */}
        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <Sparkles className="h-4 w-4 text-accent-ink" />
              <CardTitle>Skills Registry</CardTitle>
            </div>
            <Badge variant="default">{skills.length} installed</Badge>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              <Input
                value={skName}
                onChange={(e) => setSkName(e.target.value)}
                placeholder="Skill name"
              />
              <Input
                value={skDesc}
                onChange={(e) => setSkDesc(e.target.value)}
                placeholder="Short description"
              />
            </div>
            <div className="flex gap-2">
              <Input
                value={skInst}
                onChange={(e) => setSkInst(e.target.value)}
                placeholder="Instructions / prompt template..."
                className="flex-1"
              />
              <Button variant="primary" size="sm" onClick={addSkill}>
                Install Skill
              </Button>
            </div>

            {skills.length === 0 ? (
              <EmptyState title="No custom skills installed" description="Create reusable prompt skills for MCP & A2A agents." />
            ) : (
              <div className="space-y-2 max-h-80 overflow-y-auto">
                {skills.map((s: any) => (
                  <div
                    key={s.id}
                    className="flex items-start justify-between gap-3 rounded-xl border border-border bg-surface-subtle/40 p-3 text-xs"
                  >
                    <div className="min-w-0 flex-1 space-y-1">
                      <div className="font-semibold text-fg">{s.name}</div>
                      {s.description && <div className="text-fg-muted">{s.description}</div>}
                    </div>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="text-danger hover:bg-danger-bg"
                      onClick={async () => {
                        await apiDelete(`/api/skills/${encodeURIComponent(s.id)}`);
                        onToast("Skill removed", "info");
                        onRefresh();
                      }}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
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
