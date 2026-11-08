import { useCallback, useEffect, useRef, useState } from "react";
import { api, type ImageBox, type ImageItem, type ImageSpeciesRow, type Summary } from "../api";

const PER_PAGE = 24;
const FILTERS: [string, string][] = [["animals", "Animals"], ["empty", "Empty"], ["people", "People"], ["vehicles", "Vehicles"], ["todo", "Not processed"], ["failed", "Could not read"], ["all", "All"]];
const KIND_NAME: Record<string, string> = { animal: "animal", human: "person", vehicle: "vehicle", blank: "nothing", unknown: "unknown" };

/** Camera-trap images: per species, a grid of photos with the boxes SpeciesNet drew. Each answer is about one box and
 * refits that species' threshold, as for sound. Keys: Y yes, N no, U unsure, arrows to move, Enter to enlarge. Images
 * with people are never exported or published. */
export default function Images({ summary, onError }: { summary: Summary; onError: (e: string) => void }) {
  const [rows, setRows] = useState<ImageSpeciesRow[]>([]);
  const [counts, setCounts] = useState<Record<string, number>>({});
  const [sel, setSel] = useState<{ label: number; model: string } | null>(null);
  const [show, setShow] = useState("animals");
  const [items, setItems] = useState<ImageItem[]>([]);
  const [page, setPage] = useState(0);
  const [i, setI] = useState(0);
  const [big, setBig] = useState(false);
  const [filter, setFilter] = useState("");

  const loadLists = useCallback(async () => {
    try { setRows(await api.imageSpecies()); setCounts(await api.imageFilters()); } catch (e) { onError(String(e)); }
  }, [onError]);
  useEffect(() => { loadLists(); }, [loadLists]);

  const loadItems = useCallback(async () => {
    try {
      const q = sel ? { species: sel.label, model: sel.model, limit: PER_PAGE } : { show, offset: page * PER_PAGE, limit: PER_PAGE };
      setItems(await api.imageQueue(q)); setI(0);
    } catch (e) { onError(String(e)); }
  }, [sel, show, page, onError]);
  useEffect(() => { loadItems(); }, [loadItems]);

  const focusBox = (it: ImageItem | undefined): ImageBox | undefined => it?.boxes.find((b) => b.id === it.focus);
  const verdict = useCallback(async (v: "yes" | "no" | "unsure") => {
    const it = items[i];
    const b = focusBox(it);
    if (!it || !b) return;
    try {
      await api.setImageVerdict(b.id, v);
      setItems((xs) => xs.map((x, k) => (k === i ? { ...x, boxes: x.boxes.map((y) => (y.id === b.id ? { ...y, verdict: v } : y)) } : x)));
      setI((k) => Math.min(k + 1, items.length - 1));
      loadLists();
    } catch (e) { onError(String(e)); }
  }, [items, i, loadLists, onError]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement)?.tagName === "INPUT") return;
      const k = e.key.toLowerCase();
      if (k === "y") verdict("yes"); else if (k === "n") verdict("no"); else if (k === "u") verdict("unsure");
      else if (k === "arrowright" || k === "j") setI((x) => Math.min(items.length - 1, x + 1));
      else if (k === "arrowleft" || k === "k") setI((x) => Math.max(0, x - 1));
      else if (k === "arrowdown") { e.preventDefault(); setI((x) => Math.min(items.length - 1, x + 4)); }
      else if (k === "arrowup") { e.preventDefault(); setI((x) => Math.max(0, x - 4)); }
      else if (k === "enter") setBig((b) => !b); else if (k === "escape") setBig(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [verdict, items.length]);

  const c = summary.images;
  const row = rows.find((r) => sel && r.label === sel.label && r.model === sel.model) ?? null;
  const shown = rows.filter((r) => !filter || `${r.common} ${r.scientific}`.toLowerCase().includes(filter.toLowerCase()));
  const cur = items[i];
  return (
    <section className="review">
      <div className="species-list">
        <div className="wnc-card">
          <b>Camera-trap images</b>
          <small className="muted block">{c ? `${c.done.toLocaleString()} of ${c.total.toLocaleString()} processed with SpeciesNet` : "No images yet"}</small>
        </div>
        <input className="search" placeholder={`Search ${rows.length} species and groups`} value={filter} onChange={(e) => setFilter(e.target.value)} />
        <ul>
          {shown.map((r) => (
            <li key={`${r.label}|${r.model}`} className={sel && r.label === sel.label && r.model === sel.model ? "active" : ""} onClick={() => { setSel({ label: r.label, model: r.model }); setBig(false); }}>
              <div className="sp-name"><b>{r.common || r.scientific}</b><i>{r.common ? r.scientific : ""}</i></div>
              <div className="sp-meta">
                <span title="Boxes kept at the current threshold, of all">{r.kept.toLocaleString()} / {r.boxes.toLocaleString()} in {r.images.toLocaleString()} {r.images === 1 ? "image" : "images"}</span>
                {r.reviewed > 0 ? <span className={r.threshold_source === "fitted" ? "tag ok" : "tag"}>{r.reviewed} checked</span> : <span className="tag dim">not checked</span>}
              </div>
            </li>
          ))}
        </ul>
        {rows.length === 0 && <p className="muted pad">No animals named yet. Process the images first.</p>}
      </div>
      <div className="clips">
        <header className="head">
          <div>
            {row ? (
              <>
                <h2>{row.common || row.scientific} <i className="muted">{row.common ? row.scientific : ""}</i></h2>
                <p className="muted">
                  {row.threshold_source === "fitted"
                    ? row.threshold > 1 ? <>Your answers so far say none of these are reliable: none will be exported unless you mark them yes.</>
                      : <>Kept from score <b>{row.threshold.toFixed(1)}</b> up ({row.kept.toLocaleString()} boxes), estimated {Math.round((row.precision ?? 0) * 100)}% right, from {row.reviewed} answers.</>
                    : <>Threshold {row.threshold.toFixed(2)} (default) until {Math.max(0, 8 - row.reviewed)} more answers.</>}
                </p>
              </>
            ) : (
              <>
                <h2>Images</h2>
                <p className="muted">Pick a species to check its boxes, a few from every score level. Or browse by what SpeciesNet saw. Images with people stay on this computer: they are never exported or published.</p>
              </>
            )}
          </div>
          {row && <button onClick={() => loadItems()}>New images</button>}
        </header>
        <div className="tabs" style={{ marginBottom: 12 }}>
          {FILTERS.map(([k, label]) => (
            <button key={k} className={`tab ${!sel && show === k ? "on" : ""}`} onClick={() => { setSel(null); setShow(k); setPage(0); setBig(false); }}>
              {label} <span className="count">{(counts[k] ?? 0).toLocaleString()}</span>
            </button>
          ))}
        </div>
        {items.length === 0 ? <p className="muted">Nothing here.</p> : big && cur ? (
          <Enlarged it={cur} onVerdict={verdict} onClose={() => setBig(false)} />
        ) : (
          <div className="img-grid">
            {items.map((it, k) => <Tile key={`${it.id}-${it.focus}`} it={it} on={k === i} onClick={() => setI(k)} onOpen={() => { setI(k); setBig(true); }} />)}
          </div>
        )}
        {cur && !big && <TileActions it={cur} onVerdict={verdict} />}
        {!sel && <div className="row pager">
          <button disabled={page === 0} onClick={() => setPage((p) => p - 1)}>Previous</button>
          <span className="muted small">Page {page + 1}</span>
          <button disabled={items.length < PER_PAGE} onClick={() => setPage((p) => p + 1)}>Next</button>
        </div>}
        <p className="muted small">Keys: Y yes, N no, U unsure, arrows to move, Enter to enlarge.</p>
      </div>
    </section>
  );
}

function useThumb(path: string, max: number) {
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    setSrc(null); setFailed(null);
    api.imageThumb(path, max).then((b) => { if (live) setSrc(`data:image/jpeg;base64,${b}`); }).catch((e) => { if (live) setFailed(String(e)); });
    return () => { live = false; };
  }, [path, max]);
  return { src, failed };
}

function Boxes({ it }: { it: ImageItem }) {
  return (
    <>
      {it.boxes.map((b) => (
        <div key={b.id} className={`ibox k-${b.kind} ${b.id === it.focus ? "focus" : ""} ${b.verdict ?? ""}`}
          style={{ left: `${b.bbox[0] * 100}%`, top: `${b.bbox[1] * 100}%`, width: `${b.bbox[2] * 100}%`, height: `${b.bbox[3] * 100}%` }}>
          <span>{b.kind === "animal" ? (b.common || b.scientific) : KIND_NAME[b.kind]} {b.score.toFixed(2)}</span>
        </div>
      ))}
    </>
  );
}

function Tile({ it, on, onClick, onOpen }: { it: ImageItem; on: boolean; onClick: () => void; onOpen: () => void }) {
  const { src, failed } = useThumb(it.path, 480);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => { if (on) ref.current?.scrollIntoView({ block: "nearest" }); }, [on]);
  const f = it.boxes.find((b) => b.id === it.focus);
  return (
    <div ref={ref} className={`tile ${on ? "on" : ""} ${f?.verdict ?? ""}`} onClick={onClick} onDoubleClick={onOpen}>
      <div className="frame">
        {src ? <img src={src} alt="" /> : <div className="loading">{failed ? "Could not read this image" : "Loading..."}</div>}
        {src && <Boxes it={it} />}
      </div>
      <div className="tile-meta">
        <span>{it.site}</span>
        <span className="muted">{it.taken_local ?? "time unknown"}</span>
        {f?.verdict && <b className={`v-${f.verdict}`}>{f.verdict === "yes" ? "Yes" : f.verdict === "no" ? "No" : "Not sure"}</b>}
      </div>
    </div>
  );
}

function TileActions({ it, onVerdict }: { it: ImageItem; onVerdict: (v: "yes" | "no" | "unsure") => void }) {
  const f = it.boxes.find((b) => b.id === it.focus);
  const others = f?.top.filter((t) => t[0] !== (f.common || f.scientific)).slice(0, 3) ?? [];
  return (
    <div className="clip" style={{ marginTop: 12 }}>
      <div className="clip-meta">
        {f ? <span>Box: <b>{f.kind === "animal" ? (f.common || f.scientific) : KIND_NAME[f.kind]}</b> score {f.score.toFixed(2)}, detector {f.conf.toFixed(2)}</span> : <span className="muted">No box to check in this image.</span>}
        {others.length > 0 && <span className="muted">Also suggested: {others.map((t) => `${t[0]} ${t[1].toFixed(2)}`).join(", ")}</span>}
        <span className="muted mono" title={it.path}>{it.path.split(/[\\/]/).pop()}</span>
      </div>
      {f && <div className="verdicts">
        <button className={`yes ${f.verdict === "yes" ? "on" : ""}`} onClick={() => onVerdict("yes")}>Yes, it is <kbd>Y</kbd></button>
        <button className={`no ${f.verdict === "no" ? "on" : ""}`} onClick={() => onVerdict("no")}>No <kbd>N</kbd></button>
        <button className={`unsure ${f.verdict === "unsure" ? "on" : ""}`} onClick={() => onVerdict("unsure")}>Not sure <kbd>U</kbd></button>
      </div>}
    </div>
  );
}

function Enlarged({ it, onVerdict, onClose }: { it: ImageItem; onVerdict: (v: "yes" | "no" | "unsure") => void; onClose: () => void }) {
  const { src, failed } = useThumb(it.path, 1600);
  return (
    <div>
      <div className="frame big" onClick={onClose}>
        {src ? <img src={src} alt="" /> : <div className="loading">{failed ? `This image could not be read: ${failed}` : "Loading..."}</div>}
        {src && <Boxes it={it} />}
      </div>
      <TileActions it={it} onVerdict={onVerdict} />
    </div>
  );
}
