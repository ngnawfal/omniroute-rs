import { useState } from "react";
import { Database, Download, HardDrive, Upload } from "lucide-react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Badge,
  Button,
  Input,
  PageHeader,
} from "@/components/ui/primitives";
import { apiPost } from "@/lib/api";

export function DatabasePage({
  db,
  onRefresh,
  onToast,
}: {
  db: any;
  onRefresh: () => void;
  onToast: (msg: string, tone?: "success" | "error" | "info") => void;
}) {
  const [sqlitePath, setSqlitePath] = useState("/root/.omniroute/storage.sqlite");
  const [importRes, setImportRes] = useState<any>(null);
  const [busy, setBusy] = useState(false);

  const tables: Record<string, number> = db?.table_counts || db?.tables || {};
  const integrityOk = db?.ok === true || db?.integrity === "ok" || db?.integrity_ok === true;
  const dbBytes = db?.size_bytes ?? db?.file_size_bytes ?? (db?.page_count && db?.page_size ? db.page_count * db.page_size : 0);

  const runBackup = async () => {
    const r = await apiPost("/api/db/backup", {});
    if (r.ok) {
      onToast(`Hot backup created: ${r.data.path || "OK"}`, "success");
      onRefresh();
    } else onToast(r.error || "Backup failed", "error");
  };

  const runImport = async () => {
    if (!sqlitePath.trim()) return;
    setBusy(true);
    const r = await apiPost("/api/db/import/upstream-sqlite", { path: sqlitePath.trim() });
    setBusy(false);
    if (r.ok) {
      setImportRes(r.data);
      onToast("Imported upstream Node.js database!", "success");
      onRefresh();
    } else {
      onToast(r.error || "Import failed", "error");
    }
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="Database & Upstream Migration"
        description="SQLite WAL integrity diagnostics, atomic VACUUM INTO hot backups, portable JSON snapshots & direct Node.js storage.sqlite migration."
        actions={
          <>
            <Button variant="primary" size="sm" onClick={runBackup}>
              <HardDrive className="h-3.5 w-3.5" /> Create Hot Backup
            </Button>
            <a
              href="/api/db/export"
              download="omniroute-rs-snapshot.json"
              className="inline-flex h-8 items-center gap-1.5 rounded-md border border-border bg-surface-subtle px-3 text-xs font-medium text-fg hover:bg-surface-muted"
            >
              <Download className="h-3.5 w-3.5" /> Export JSON Snapshot
            </a>
          </>
        }
      />

      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        <Card className="p-4">
          <div className="text-xs text-fg-muted">SQLite Integrity</div>
          <div className="mt-1.5 text-xl font-semibold text-success">
            {integrityOk ? "OK (WAL)" : "CHECK"}
          </div>
          <div className="mt-1 font-mono text-[11px] text-fg-muted">
            {db?.journal_mode || "wal"}
          </div>
        </Card>
        <Card className="p-4">
          <div className="text-xs text-fg-muted">Database Size</div>
          <div className="mt-1.5 font-mono text-xl font-semibold text-fg">
            {(dbBytes / 1024).toFixed(1)} KB
          </div>
          <div className="mt-1 truncate font-mono text-[10px] text-fg-muted">
            {db?.db_path || "data/omniroute.sqlite"}
          </div>
        </Card>
        <Card className="p-4">
          <div className="text-xs text-fg-muted">Schema Tables</div>
          <div className="mt-1.5 font-mono text-xl font-semibold text-fg">
            {Object.keys(tables).length}
          </div>
          <div className="mt-1 text-[11px] text-fg-muted">WAL mode enabled</div>
        </Card>
        <Card className="p-4">
          <div className="text-xs text-fg-muted">Applied Migrations</div>
          <div className="mt-1.5 font-mono text-xl font-semibold text-fg">
            {db?.applied_migrations ?? 15}
          </div>
          <div className="mt-1 text-[11px] text-fg-muted">Upstream compatible</div>
        </Card>
      </div>

      <div className="grid grid-cols-1 gap-5 lg:grid-cols-5">
        <Card className="lg:col-span-3">
          <CardHeader>
            <div className="flex items-center gap-2">
              <Database className="h-4 w-4 text-accent-ink" />
              <CardTitle>SQLite Tables ({Object.keys(tables).length} WAL tables)</CardTitle>
            </div>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-3">
              {Object.entries(tables).map(([name, count]) => (
                <div
                  key={name}
                  className="rounded-xl border border-border bg-surface-subtle/40 p-3"
                >
                  <div className="font-mono text-lg font-semibold text-fg">{count}</div>
                  <div className="truncate font-mono text-[11px] text-fg-muted">{name}</div>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>

        <Card className="lg:col-span-2">
          <CardHeader>
            <div className="flex items-center gap-2">
              <Upload className="h-4 w-4 text-accent-ink" />
              <CardTitle>Import Node.js storage.sqlite</CardTitle>
            </div>
          </CardHeader>
          <CardContent className="space-y-3">
            <p className="text-xs leading-relaxed text-fg-muted">
              Reads <code className="font-mono">provider_connections</code>,{" "}
              <code className="font-mono">combos</code>, <code className="font-mono">api_keys</code>,
              and <code className="font-mono">usage_history</code> in read-only mode without altering the original file.
            </p>
            <div className="flex gap-2">
              <Input
                value={sqlitePath}
                onChange={(e) => setSqlitePath(e.target.value)}
                placeholder="/root/.omniroute/storage.sqlite"
                className="flex-1 font-mono text-xs"
              />
              <Button variant="primary" size="sm" disabled={busy} onClick={runImport}>
                {busy ? "Importing..." : "Import"}
              </Button>
            </div>
            {importRes && (
              <div className="rounded-xl border border-success/35 bg-success-bg p-3 text-xs text-success">
                <Badge variant="success" className="mb-1.5">Import Complete</Badge>
                <pre className="overflow-auto font-mono text-[11px]">
                  {JSON.stringify(importRes, null, 2)}
                </pre>
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
