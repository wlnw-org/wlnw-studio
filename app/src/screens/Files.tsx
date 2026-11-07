import { useCallback, useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { api, fmtBytes, fmtDuration, fmtWhen, modelTag, type FilePage, type FileStatus, type Summary } from "../api";

const PAGE = 200;
const TABS: { id: FileStatus | ""; label: string }[] = [
  { id: "", label: "All" }, { id: "done", label: "Done" }, { id: "partial", label: "Partly done" },
  { id: "todo", label: "To do" }, { id: "failed", label: "Could not read" }, { id: "none", label: "No model" },
];
const STATUS_TEXT: Record<FileStatus, string> = { done: "Done", partial: "Partly done", todo: "To do", failed: "Could not read", none: "No model" };

/** Every recording in the project and where it stands, per model. Failed ones show why and can be tried again. */
export default function Files({ summary, refresh, onError, initialStatus }: { summary: Summary; refresh: () => void; onError: (e: string) => void; initialStatus?: string }) {
  const [status, setStatus] = useState<string>(initialStatus ?? "");
  const [dep, setDep] = useState<number | null>(null);
  const [search, setSearch] = useState("");
  const [offset, setOffset] = useState(0);
  const [page, setPage] = useState<FilePage | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const query = { status: status || null, deployment: dep, search: search.trim() || null };

  const load = useCallback(async () => {
    try { setPage(await api.listFiles({ ...query, offset, limit: PAGE })); } catch (e) { onError(String(e)); }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [status, dep, search, offset, onError]);

  useEffect(() => { const t = setTimeout(load, search ? 250 : 0); return () => clearTimeout(t); }, [load, search]);
  // Follow a run while it goes.
  useEffect(() => { if (!summary.running) return; const t = setInterval(load, 4000); return () => clearInterval(t); }, [summary.running, load]);
  useEffect(() => { setOffset(0); }, [status, dep, search]);

  async function retry(file: number | null) {
    try {
      const n = await api.retryFailed(file);
      setMsg(n === 0 ? "Nothing to try again." : `${n.toLocaleString()} failed ${n === 1 ? "attempt" : "attempts"} cleared. Start processing to try ${file == null ? "them" : "it"} again.`);
      refresh(); load();
    } catch (e) { onError(String(e)); }
  }
  async function saveList() {
    const out = await save({ title: "Save the list of recordings", defaultPath: `${summary.name}-recordings.csv`, filters: [{ name: "CSV", extensions: ["csv"] }] });
    if (!out) return;
    try { const n = await api.exportFileList(query, out); setMsg(`${n.toLocaleString()} recordings saved to ${out}.`); } catch (e) { onError(String(e)); }
  }

  const c = page?.counts;
  const name = (p: string) => p.split(/[\\/]/).pop() ?? p;
  const folder = (p: string) => p.split(/[\\/]/).slice(-3, -1).join("/");
  return (
    <section>
      <header className="head">
        <div>
          <h2>Files</h2>
          <p className="muted">Every recording in this project and how far it has got with each model. Recordings that could not be read show why.</p>
        </div>
        <div className="row tight">
          {c && c.failed > 0 && <button onClick={() => retry(null)} disabled={summary.running} title={summary.running ? "Pause processing first" : ""}>Try failed ones again</button>}
          <button onClick={saveList} disabled={!page || page.total === 0}>Save list as CSV</button>
        </div>
      </header>
      {msg && <p className="note" onClick={() => setMsg(null)}>{msg}</p>}

      <div className="filters">
        <div className="tabs" role="tablist">
          {TABS.filter((t) => t.id !== "none" || (c?.none ?? 0) > 0).map((t) => (
            <button key={t.id} role="tab" aria-selected={status === t.id} className={status === t.id ? "tab on" : "tab"} onClick={() => setStatus(t.id)}>
              {t.label} <span className="count">{c ? (t.id === "" ? c.all : c[t.id]).toLocaleString() : ""}</span>
            </button>
          ))}
        </div>
        <div className="row tight">
          {summary.deployments.length > 1 && (
            <select value={dep ?? ""} onChange={(e) => setDep(e.target.value ? Number(e.target.value) : null)} aria-label="Recorder">
              <option value="">All recorders</option>
              {summary.deployments.map((d) => <option key={d.id} value={d.id}>{d.name}</option>)}
            </select>
          )}
          <input type="search" value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Search file or folder name" aria-label="Search" />
        </div>
      </div>

      {page && page.rows.length === 0 ? (
        <div className="empty">{c?.all ? "No recordings match." : "No recordings yet. Add a folder or drive under Recordings."}</div>
      ) : page && (
        <>
          <table className="grid files">
            <thead><tr><th>Recording</th><th>Recorder</th><th>Recorded</th><th>Length</th><th>Status</th><th>Detections</th><th /></tr></thead>
            <tbody>
              {page.rows.map((r) => (
                <tr key={r.id}>
                  <td className="fname"><b title={r.path}>{name(r.path)}</b><small className="muted block">{folder(r.path)} · {fmtBytes(r.bytes)}</small></td>
                  <td>{r.recorder}</td>
                  <td className="small">{r.start_local ?? <span className="muted">unknown</span>}</td>
                  <td className="small">{r.duration_s != null ? fmtDuration(r.duration_s) : ""}</td>
                  <td>
                    <span className={`status st-${r.status}`}>{STATUS_TEXT[r.status]}</span>
                    <div className="model-states">
                      {r.models.map((m) => (
                        <small key={m.model} className={`ms ms-${m.status}`} title={m.processed_at ? `Processed ${fmtWhen(m.processed_at)}` : "Not processed yet"}>
                          {modelTag(m.model)}: {m.status === "done" ? "done" : m.status === "error" ? "failed" : "to do"}
                        </small>
                      ))}
                    </div>
                    {r.models.filter((m) => m.error).map((m) => <small key={m.model} className="error block">{m.error!.split(`${r.path}: `).join("")}</small>)}
                  </td>
                  <td>{r.detections > 0 ? r.detections.toLocaleString() : <span className="muted">0</span>}</td>
                  <td className="actions">
                    <button className="small-btn" onClick={() => revealItemInDir(r.path).catch((e) => onError(String(e)))}>Show</button>
                    {r.status === "failed" && <button className="small-btn" onClick={() => retry(r.id)} disabled={summary.running}>Retry</button>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {page.total > PAGE && (
            <div className="row pager">
              <button onClick={() => setOffset(Math.max(0, offset - PAGE))} disabled={offset === 0}>Previous</button>
              <span className="muted">{(offset + 1).toLocaleString()} to {Math.min(offset + PAGE, page.total).toLocaleString()} of {page.total.toLocaleString()}</span>
              <button onClick={() => setOffset(offset + PAGE)} disabled={offset + PAGE >= page.total}>Next</button>
            </div>
          )}
        </>
      )}
    </section>
  );
}
