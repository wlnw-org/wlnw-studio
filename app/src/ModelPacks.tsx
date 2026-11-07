import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { api, fmtBytes, type CatalogItem, type DownloadEvent } from "./api";

/** The models on this computer: download Perch on first run, add BirdNET's files from a folder, see what is installed. */
export default function ModelPacks({ onChange, onError }: { onChange: () => void; onError: (e: string) => void }) {
  const [items, setItems] = useState<CatalogItem[]>([]);
  const [prog, setProg] = useState<Record<string, DownloadEvent>>({});
  const load = useCallback(async () => { try { setItems(await api.modelCatalog()); } catch (e) { onError(String(e)); } }, [onError]);
  useEffect(() => { load(); }, [load]);
  useEffect(() => {
    const un = listen<DownloadEvent>("model-progress", (e) => {
      const d = e.payload;
      if (d.finished) {
        setProg((p) => { const n = { ...p }; delete n[d.id]; return n; });
        if (d.error) onError(d.error);
        load(); onChange();
      } else setProg((p) => ({ ...p, [d.id]: d }));
    });
    return () => { un.then((f) => f()); };
  }, [load, onChange, onError]);

  async function download(id: string) { try { await api.downloadModel(id); load(); } catch (e) { onError(String(e)); } }
  async function fromFolder() {
    const f = await open({ title: "Folder with Studio model files", directory: true, multiple: false });
    if (typeof f !== "string") return;
    try { await api.addModelsFolder(f); load(); onChange(); } catch (e) { onError(String(e)); }
  }
  return (
    <div className="card">
      <h3>Models on this computer</h3>
      <div className="packs">
        {items.map((m) => {
          const p = prog[m.id];
          const pct = p && p.total ? Math.floor((p.done / p.total) * 100) : 0;
          return (
            <div key={m.id} className="pack">
              <div className="pack-text">
                <b>{m.name}</b> <small className="muted">licence <a href={m.licence_url} target="_blank" rel="noreferrer">{m.licence}</a></small>
                <small className="block muted">{m.description}</small>
                {!m.downloadable && !m.installed && <small className="block warn">{m.note}</small>}
                {(m.downloading || p) && <div className="bar slim"><div style={{ width: `${pct}%` }} /></div>}
              </div>
              <div className="pack-act">
                {m.installed ? <span className="ok">Installed</span>
                  : m.downloading || p ? <><span className="small">{p ? `${fmtBytes(p.done)} of ${fmtBytes(p.total)}` : "Starting..."}</span><button className="small-btn" onClick={() => api.cancelDownload()}>Pause</button></>
                  : m.downloadable ? <button className="primary" onClick={() => download(m.id)}>Download ({fmtBytes(m.size)})</button>
                  : null}
              </div>
            </div>
          );
        })}
      </div>
      <p className="muted small">Downloads are checked against a fixed checksum before use, and a paused or interrupted download continues where it stopped. <button className="link inline" onClick={fromFolder}>Add model files from a folder</button></p>
    </div>
  );
}
