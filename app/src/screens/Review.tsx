import { useCallback, useEffect, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, modelName, modelTag, type Clip, type Detection, type Similar, type SpeciesRow, type Summary } from "../api";

const PER_ROUND = 18;

/** Species first, per model. Each round shows clips from every confidence band; each verdict refits that species'
 * threshold. Any clip can be named ("this is X") and used to find more like it, which teaches the Wildlife Network Classifier.
 * Keys: Y yes, N no, U unsure, Space play, arrow keys or J / K to move. */
export default function Review({ summary, refresh, onError }: { summary: Summary; refresh: () => void; onError: (e: string) => void }) {
  const [rows, setRows] = useState<SpeciesRow[]>([]);
  const [sel, setSel] = useState<{ label: number; model: string } | null>(null);
  const [queue, setQueue] = useState<Detection[]>([]);
  const [i, setI] = useState(0);
  const [filter, setFilter] = useState("");
  const [training, setTraining] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const load = useCallback(async () => { try { setRows(await api.species()); } catch (e) { onError(String(e)); } }, [onError]);
  useEffect(() => { load(); }, [load]);

  async function pick(label: number, model: string) {
    setSel({ label, model }); setI(0);
    try { setQueue(await api.reviewQueue(label, model, PER_ROUND)); } catch (e) { onError(String(e)); }
  }
  const verdict = useCallback(async (v: "yes" | "no" | "unsure") => {
    const d = queue[i];
    if (!d) return;
    try {
      await api.setVerdict(d.id, v);
      setQueue((q) => q.map((x, k) => (k === i ? { ...x, verdict: v } : x)));
      setI((k) => Math.min(k + 1, queue.length - 1));
      load();
    } catch (e) { onError(String(e)); }
  }, [queue, i, load, onError]);

  async function train() {
    setTraining(true); setNote(null);
    try {
      const r = await api.trainWnc();
      setNote(r.classes.length
        ? `Trained ${r.classes.length} ${r.classes.length === 1 ? "species" : "species"}; ${r.detections.toLocaleString()} detections found.` + (r.skipped.length ? ` Not enough examples yet for: ${r.skipped.map((s) => `${s[0]} (${s[1]})`).join(", ")}.` : "")
        : "Not enough examples yet: each species needs at least 3 clips marked yes (or named) in recordings processed with Perch.");
      refresh(); load();
    } catch (e) { onError(String(e)); }
    setTraining(false);
  }
  async function share() {
    const out = await save({ title: "Save the classifier to share", defaultPath: "wildlife-network-classifier.wnc", filters: [{ name: "Wildlife Network Classifier", extensions: ["wnc"] }] });
    if (out) try { await api.wncExport(out); setNote(`Saved to ${out}.`); } catch (e) { onError(String(e)); }
  }
  async function importPack() {
    const f = await open({ title: "Add a shared classifier", multiple: false, filters: [{ name: "Wildlife Network Classifier", extensions: ["wnc"] }] });
    if (typeof f === "string") try { const n = await api.wncImport(f); setNote(`Added. ${n.toLocaleString()} detections found.`); refresh(); load(); } catch (e) { onError(String(e)); }
  }

  const row = rows.find((r) => sel && r.label === sel.label && r.model === sel.model) ?? null;
  const shown = rows.filter((r) => !filter || `${r.common} ${r.scientific}`.toLowerCase().includes(filter.toLowerCase()));
  const done = queue.filter((d) => d.verdict).length;
  const hasPerch = summary.deployments.some((d) => d.models.includes("perch-2.0"));
  return (
    <section className="review">
      <div className="species-list">
        <div className="wnc-card">
          <b>Wildlife Network Classifier</b>
          <small className="muted block">{summary.wnc ? `${summary.wnc.classes.length} species learned from your reviews` : "Learns your local species from your answers"}</small>
          <div className="row tight">
            <button className="small-btn" onClick={train} disabled={training || !hasPerch} title={hasPerch ? "" : "Needs recordings processed with Perch"}>{training ? "Training..." : "Train"}</button>
            {summary.wnc && <button className="small-btn" onClick={share}>Share</button>}
            <button className="small-btn" onClick={importPack}>Add shared</button>
          </div>
          {note && <small className="block note-sm">{note}</small>}
        </div>
        <input className="search" placeholder={`Search ${rows.length} species`} value={filter} onChange={(e) => setFilter(e.target.value)} />
        <ul>
          {shown.map((r) => (
            <li key={`${r.label}|${r.model}`} className={sel && r.label === sel.label && r.model === sel.model ? "active" : ""} onClick={() => pick(r.label, r.model)}>
              <div className="sp-name"><b>{r.common || r.scientific}</b><i>{r.common ? r.scientific : ""}</i></div>
              <div className="sp-meta">
                <span title="Detections kept at the current threshold, of all in range">{r.kept.toLocaleString()} / {r.detections.toLocaleString()}</span>
                <span className={`tag model-${r.model}`} title={modelName(r.model)}>{modelTag(r.model)}</span>
                {r.reviewed > 0 ? <span className={r.threshold_source === "fitted" ? "tag ok" : "tag"}>{r.reviewed} checked</span> : <span className="tag dim">not checked</span>}
              </div>
            </li>
          ))}
        </ul>
        {rows.length === 0 && <p className="muted pad">No detections yet. Process some recordings first.</p>}
      </div>
      <div className="clips">
        {!row ? (
          <div className="empty">
            <h2>Review</h2>
            <p>Pick a species. You hear a few clips from every confidence level, from sure to doubtful. Mark each one. After 8 answers Studio sets that species&apos; threshold so that what is kept is right at least 90% of the time.</p>
            <p>Hear something the models got wrong, or a species they do not know? Use <b>Name this sound</b> on any clip, then <b>Find more like this</b>. Train the Wildlife Network Classifier and it will look for that species everywhere.</p>
            <p className="muted">Keys: Y yes, N no, U unsure, Space play, arrows to move.</p>
          </div>
        ) : (
          <>
            <header className="head">
              <div>
                <h2>{row.common || row.scientific} <i className="muted">{row.common ? row.scientific : ""}</i> <span className={`tag model-${row.model}`}>{modelName(row.model)}</span></h2>
                <p className="muted">
                  {row.threshold_source === "fitted"
                    ? row.threshold > 1 ? <>Your answers so far say none of these are reliable: none will be exported unless you mark them yes.</>
                      : <>Kept from confidence <b>{row.threshold.toFixed(1)}</b> up ({row.kept.toLocaleString()} detections), estimated {Math.round((row.precision ?? 0) * 100)}% right, from {row.reviewed} answers.</>
                    : <>Threshold {row.threshold.toFixed(2)} (default) until {Math.max(0, 8 - row.reviewed)} more answers.</>}
                </p>
              </div>
              <button onClick={() => pick(row.label, row.model)}>{done === queue.length && queue.length ? "Next round" : "New clips"}</button>
            </header>
            {queue.length === 0 ? <p className="muted">Every detection of this species has been reviewed.</p> : (
              <>
                <ClipView key={queue[i].id} d={queue[i]} onVerdict={verdict} onMove={(k) => setI((x) => Math.max(0, Math.min(queue.length - 1, x + k)))} />
                <div className="strip">
                  {queue.map((d, k) => (
                    <button key={d.id} className={`dot ${k === i ? "cur" : ""} ${d.verdict ?? ""}`} onClick={() => setI(k)} title={`confidence ${d.confidence.toFixed(2)}`}>{d.confidence.toFixed(2)}</button>
                  ))}
                </div>
                <Teach key={`t${queue[i].id}`} d={queue[i]} onError={onError} />
              </>
            )}
          </>
        )}
      </div>
    </section>
  );
}

/** Name the sound in this clip (any species, even one no model knows) and find more like it. */
function Teach({ d, onError }: { d: Detection; onError: (e: string) => void }) {
  const [name, setName] = useState("");
  const [common, setCommon] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const [similar, setSimilar] = useState<Similar[] | null>(null);
  const [marked, setMarked] = useState<Record<number, boolean>>({});
  const sci = name.trim() || d.scientific;
  async function nameIt(present: boolean) {
    try {
      const ok = await api.nameSound(d.id, sci, common.trim() || (name.trim() ? "" : d.common), present);
      setMsg(ok ? (present ? `Saved: this clip is ${sci}.` : `Saved: ${sci} is not in this clip.`) : "This recording has not been through Perch yet, so it cannot teach the classifier. Run Perch on it first.");
    } catch (e) { onError(String(e)); }
  }
  async function findMore() {
    try { setSimilar(await api.similar(d.id, 12)); } catch (e) { onError(String(e)); }
  }
  async function mark(w: Similar, present: boolean) {
    try { await api.labelWindow(w.window_id, sci, common.trim(), present); setMarked({ ...marked, [w.window_id]: present }); } catch (e) { onError(String(e)); }
  }
  return (
    <details className="card teach">
      <summary>Name this sound, or find more like it</summary>
      <p className="muted small">For the Wildlife Network Classifier. Use the scientific name; it can be a species BirdNET and Perch do not know.</p>
      <div className="row">
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder={d.scientific} aria-label="Scientific name" />
        <input value={common} onChange={(e) => setCommon(e.target.value)} placeholder="Common name (optional)" aria-label="Common name" />
      </div>
      <div className="row tight">
        <button onClick={() => nameIt(true)}>This is {sci}</button>
        <button onClick={() => nameIt(false)}>{sci} is not here</button>
        <button onClick={findMore}>Find more like this</button>
      </div>
      {msg && <p className="note">{msg}</p>}
      {similar && (
        <div className="similar">
          <p className="muted small">The most similar 5 s windows in this project. Listen and mark the ones that are {sci}; then train the classifier.</p>
          {similar.length === 0 && <p className="muted">Nothing similar found.</p>}
          {similar.map((w) => <SimilarRow key={w.window_id} w={w} sci={sci} mark={marked[w.window_id]} onMark={(p) => mark(w, p)} onError={onError} />)}
        </div>
      )}
    </details>
  );
}

function SimilarRow({ w, sci, mark, onMark, onError }: { w: Similar; sci: string; mark: boolean | undefined; onMark: (p: boolean) => void; onError: (e: string) => void }) {
  const a = useRef<HTMLAudioElement | null>(null);
  async function play() {
    try {
      if (!a.current) { const c = await api.clip(w.path, w.start_s, w.end_s); a.current = new Audio(`data:audio/wav;base64,${c.wav}`); }
      a.current.currentTime = 0; a.current.play();
    } catch (e) { onError(String(e)); }
  }
  return (
    <div className="sim-row">
      <button className="small-btn" onClick={play}>Play</button>
      <span className="mono small">{Math.round(w.similarity * 100)}%</span>
      <span className="small">{w.site}, {w.path.split(/[\\/]/).pop()} at {w.start_s.toFixed(0)} s</span>
      <span className="row tight">
        <button className={`small-btn ${mark === true ? "on-yes" : ""}`} onClick={() => onMark(true)}>{sci}</button>
        <button className={`small-btn ${mark === false ? "on-no" : ""}`} onClick={() => onMark(false)}>Not it</button>
      </span>
    </div>
  );
}

// Perceptual colour map (dark blue to yellow), so faint calls stay visible.
const LUT = Array.from({ length: 256 }, (_, v) => {
  const t = v / 255;
  const r = Math.round(255 * Math.min(1, Math.max(0, 1.6 * t - 0.35)));
  const g = Math.round(255 * Math.min(1, Math.max(0, 1.3 * t - 0.1)) ** 1.2);
  const b = Math.round(255 * Math.min(1, Math.max(0.12, 0.55 + 0.6 * Math.sin(Math.PI * t) - 0.5 * t)));
  return [r, g, b];
});

function ClipView({ d, onVerdict, onMove }: { d: Detection; onVerdict: (v: "yes" | "no" | "unsure") => void; onMove: (k: number) => void }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const audio = useRef<HTMLAudioElement | null>(null);
  const [clip, setClip] = useState<Clip | null>(null);
  const [playing, setPlaying] = useState(false);
  const [t, setT] = useState(0);
  const [failed, setFailed] = useState<string | null>(null);

  useEffect(() => {
    let off = false;
    api.clip(d.path, d.start_s, d.end_s).then((c) => {
      if (off) return;
      setClip(c);
      const a = new Audio(`data:audio/wav;base64,${c.wav}`);
      a.ontimeupdate = () => setT(a.currentTime);
      a.onended = () => setPlaying(false);
      audio.current = a;
      a.play().then(() => setPlaying(true)).catch(() => {});
    }).catch((e) => { if (!off) setFailed(String(e)); });
    return () => { off = true; audio.current?.pause(); };
  }, [d.path, d.start_s, d.end_s]);

  useEffect(() => {
    if (!clip || !canvas.current) return;
    const bytes = Uint8Array.from(atob(clip.spec), (ch) => ch.charCodeAt(0));
    const ctx = canvas.current.getContext("2d")!;
    canvas.current.width = clip.width; canvas.current.height = clip.height;
    const img = ctx.createImageData(clip.width, clip.height);
    for (let k = 0; k < bytes.length; k++) { const [r, g, b] = LUT[bytes[k]]; img.data.set([r, g, b, 255], k * 4); }
    ctx.putImageData(img, 0, 0);
  }, [clip]);

  const toggle = useCallback(() => {
    const a = audio.current; if (!a) return;
    if (a.paused) { if (a.ended) a.currentTime = 0; a.play(); setPlaying(true); } else { a.pause(); setPlaying(false); }
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement)?.tagName === "INPUT") return;
      const k = e.key.toLowerCase();
      if (k === "y") onVerdict("yes"); else if (k === "n") onVerdict("no"); else if (k === "u") onVerdict("unsure");
      else if (k === " ") { e.preventDefault(); toggle(); }
      else if (k === "arrowright" || k === "j") onMove(1); else if (k === "arrowleft" || k === "k") onMove(-1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onVerdict, onMove, toggle]);

  const total = clip?.total_s || 5;
  const box = clip ? { left: `${(clip.offset_s / total) * 100}%`, width: `${(clip.window_s / total) * 100}%` } : {};
  return (
    <div className="clip">
      <div className="spec" onClick={toggle}>
        {clip ? <canvas ref={canvas} /> : <div className="loading">{failed ? `This clip could not be read: ${failed}` : "Loading clip..."}</div>}
        {clip && <div className="window" style={box} />}
        {clip && <div className="cursor" style={{ left: `${(t / total) * 100}%` }} />}
        {clip && <span className="khz">{clip.max_khz} kHz</span>}
      </div>
      <div className="clip-meta">
        <span>Confidence <b>{d.confidence.toFixed(2)}</b></span>
        <span>{d.site}</span>
        <span>{d.utc ? new Date(d.utc + "Z").toLocaleString() : "time unknown"}</span>
        <span className="muted mono" title={d.path}>{d.path.split(/[\\/]/).pop()} at {d.start_s.toFixed(0)} s</span>
      </div>
      <div className="verdicts">
        <button onClick={toggle}>{playing ? "Pause" : "Play"} <kbd>Space</kbd></button>
        <button className={`yes ${d.verdict === "yes" ? "on" : ""}`} onClick={() => onVerdict("yes")}>Yes, it is <kbd>Y</kbd></button>
        <button className={`no ${d.verdict === "no" ? "on" : ""}`} onClick={() => onVerdict("no")}>No <kbd>N</kbd></button>
        <button className={`unsure ${d.verdict === "unsure" ? "on" : ""}`} onClick={() => onVerdict("unsure")}>Not sure <kbd>U</kbd></button>
      </div>
    </div>
  );
}
