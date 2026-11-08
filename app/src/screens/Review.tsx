import { useCallback, useEffect, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, isImported, modelName, modelTag, type AgreementReport, type Clip, type Detection, type Similar, type SpeciesRow, type Summary } from "../api";

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
  const [pane, setPane] = useState<"clips" | "agree">("clips");

  const load = useCallback(async () => { try { setRows(await api.species()); } catch (e) { onError(String(e)); } }, [onError]);
  useEffect(() => { load(); }, [load]);

  async function pick(label: number, model: string) {
    setSel({ label, model }); setI(0); setPane("clips");
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
        <SecondReviewer onCompare={() => setPane("agree")} onError={onError} />
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
        {pane === "agree" ? <Agreement onChanged={load} onError={onError} /> : !row ? (
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

/** Two people review the same project separately: save my answers for the other person, load theirs, compare. */
function SecondReviewer({ onCompare, onError }: { onCompare: () => void; onError: (e: string) => void }) {
  const [name, setName] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  useEffect(() => { api.reviewerName().then((n) => setName(n ?? "")).catch(() => {}); }, []);
  async function saveMine() {
    const out = await save({ title: "Save your review answers", defaultPath: `${name.trim().replace(/[^\w-]+/g, "-") || "my"}-answers.wlnr`, filters: [{ name: "Review answers", extensions: ["wlnr"] }] });
    if (out) try { const n = await api.answersExport(out, name.trim() || null); setMsg(`${n.toLocaleString()} answers saved. Send the file to the other reviewer.`); } catch (e) { onError(String(e)); }
  }
  async function loadOther() {
    const f = await open({ title: "Load another reviewer's answers", multiple: false, filters: [{ name: "Review answers", extensions: ["wlnr"] }] });
    if (typeof f !== "string") return;
    try {
      const r = await api.answersLoad(f);
      setMsg(`${r.matched.toLocaleString()} of ${r.answers.toLocaleString()} answers from ${r.reviewer} matched detections here.` + (r.unmatched ? ` ${r.unmatched.toLocaleString()} did not: their copy may have been processed differently.` : ""));
      onCompare();
    } catch (e) { onError(String(e)); }
  }
  return (
    <details className="wnc-card">
      <summary><b>Second reviewer</b></summary>
      <small className="muted block">Two people review the same recordings on their own, then compare. Your own answers are never overwritten.</small>
      <input className="search" value={name} onChange={(e) => setName(e.target.value)} placeholder="Your name (optional)" aria-label="Your name" />
      <div className="row tight">
        <button className="small-btn" onClick={saveMine}>Save my answers</button>
        <button className="small-btn" onClick={loadOther}>Load other answers</button>
        <button className="small-btn" onClick={onCompare}>Compare</button>
      </div>
      {msg && <small className="block note-sm">{msg}</small>}
    </details>
  );
}

const pct = (v: number | null) => (v == null ? "" : `${Math.round(v)}%`);

/** Agreement per species with the other reviewer, and the disagreements to settle one by one. */
function Agreement({ onChanged, onError }: { onChanged: () => void; onError: (e: string) => void }) {
  const [rep, setRep] = useState<AgreementReport | null>(null);
  const [cur, setCur] = useState<number | null>(null);
  const load = useCallback(async (who: string | null) => { try { setRep(await api.agreementReport(who)); } catch (e) { onError(String(e)); } }, [onError]);
  useEffect(() => { load(null); }, [load]);
  const list = rep?.disagreements ?? [];
  const d = cur != null ? list[cur] : undefined;
  const settle = useCallback(async (v: "yes" | "no" | "unsure") => {
    if (!rep?.reviewer || !d) return;
    try {
      await api.agreementSettle(rep.reviewer, d.id, v);
      const next = await api.agreementReport(rep.reviewer);
      setRep(next);
      setCur(next.disagreements.length ? Math.min(cur ?? 0, next.disagreements.length - 1) : null);
      onChanged();
    } catch (e) { onError(String(e)); }
  }, [rep, d, cur, onChanged, onError]);
  const move = useCallback((k: number) => setCur((x) => Math.max(0, Math.min(list.length - 1, (x ?? 0) + k))), [list.length]);
  if (!rep) return <p className="muted">Loading...</p>;
  if (!rep.reviewer) return (
    <div className="empty"><h2>Second reviewer</h2>
      <p>Ask a colleague to open a copy of this project, review on their own, and use <b>Save my answers</b>. Load their file with <b>Load other answers</b> to see where you agree, species by species.</p></div>
  );
  const who = rep.reviewer;
  const sp = (x: { common: string; scientific: string }) => x.common || x.scientific;
  return (
    <>
      <header className="head">
        <div>
          <h2>Agreement with {who}</h2>
          <p className="muted">Detections you both reviewed. Agreement and kappa count yes and no answers only (not sure is left out). Kappa corrects for agreeing by chance: 1 is full agreement, 0 is no better than chance. Settling a disagreement sets the final answer but does not change these figures.</p>
        </div>
        {rep.reviewers.length > 1 && <select value={who} onChange={(e) => { setCur(null); load(e.target.value); }} aria-label="Reviewer">{rep.reviewers.map((r) => <option key={r}>{r}</option>)}</select>}
      </header>
      {d ? (
        <div className="card">
          <p><b>{sp(d)}</b> <span className={`tag model-${d.model}`}>{modelTag(d.model)}</span> You said <b>{d.verdict ?? "nothing"}</b>, {who} said <b>{d.other}</b>. Listen again and choose the final answer. ({(cur ?? 0) + 1} of {list.length})</p>
          <ClipView key={d.id} d={d} onVerdict={settle} onMove={move} />
          <div className="row tight"><button onClick={() => setCur(null)}>Back to the table</button></div>
        </div>
      ) : (
        <>
          <table className="grid">
            <thead><tr><th>Species</th><th title="Detections you both reviewed">Both reviewed</th><th title="Both answered yes or no">Yes or no</th><th>Agree</th><th>Kappa</th></tr></thead>
            <tbody>
              {[rep.overall, ...rep.species].map((x, k) => (
                <tr key={x.scientific} className={k === 0 ? "total" : ""}>
                  <td>{k === 0 ? <b>All species</b> : <>{sp(x)} {x.common && <i className="muted">{x.scientific}</i>}</>}</td>
                  <td>{x.both}</td><td title={`Both yes ${x.yy}, both no ${x.nn}, you yes and ${who} no ${x.yn}, you no and ${who} yes ${x.ny}`}>{x.yes_no}</td>
                  <td>{pct(x.percent)}</td>
                  <td title={x.kappa == null && x.yes_no ? "Not defined: you both gave the same answer every time" : ""}>{x.kappa == null ? (x.yes_no ? "n/a" : "") : x.kappa.toFixed(2)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="card disagree">
            <h3>Disagreements to settle ({list.length}){rep.settled ? <small className="muted"> {rep.settled} settled</small> : null}</h3>
            {list.length === 0 ? <p className="muted">None left.</p> : (
              <>
                <div className="row tight"><button className="primary" onClick={() => setCur(0)}>Settle them one by one</button></div>
                {list.map((x, k) => (
                  <div key={x.id} className="sim-row">
                    <button className="small-btn" onClick={() => setCur(k)}>Open</button>
                    <span className="small"><b>{sp(x)}</b> {modelTag(x.model)}, {x.path.split(/[\\/]/).pop()} at {x.start_s.toFixed(0)} s</span>
                    <span className="small">You: {x.verdict}, {who}: {x.other}</span>
                  </div>
                ))}
              </>
            )}
          </div>
        </>
      )}
    </>
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
        {/* Imported results have no Perch embeddings to search. */}
        {!isImported(d.model) && <button onClick={findMore}>Find more like this</button>}
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
    api.clip(d.path, d.start_s, d.end_s, d.model).then((c) => {
      if (off) return;
      setClip(c);
      const a = new Audio(`data:audio/wav;base64,${c.wav}`);
      a.ontimeupdate = () => setT(a.currentTime / (c.playback || 1));
      a.onended = () => setPlaying(false);
      audio.current = a;
      a.play().then(() => setPlaying(true)).catch(() => {});
    }).catch((e) => { if (!off) setFailed(String(e)); });
    return () => { off = true; audio.current?.pause(); };
  }, [d.path, d.start_s, d.end_s, d.model]);

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
        {clip && clip.slowed > 1 && <span className="slowed">played {Math.round(clip.slowed)}x slower</span>}
      </div>
      <div className="clip-meta">
        {d.detection_score != null
          ? <><span>Species <b>{d.confidence.toFixed(2)}</b></span><span>Bat call <b>{d.detection_score.toFixed(2)}</b></span></>
          : <span>Confidence <b>{d.confidence.toFixed(2)}</b></span>}
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
