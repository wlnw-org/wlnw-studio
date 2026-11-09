import { useCallback, useEffect, useRef, useState } from "react";
import { FlagMark, SpeciesBox, useOnlineFlags, usePrefetch } from "../Online";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, isImported, modelName, modelTag, type AgreementReport, type Clip, type Detection, type Similar, type SpeciesRow, type Summary } from "../api";
import { t as tr, useT } from "../i18n";

const PER_ROUND = 18;

/** Species first, per model. Each round shows clips from every confidence band; each verdict refits that species'
 * threshold. Any clip can be named ("this is X") and used to find more like it, which teaches the Wildlife Network Classifier.
 * Keys: Y yes, N no, U unsure, Space play, arrow keys or J / K to move. */
export default function Review({ summary, refresh, onError }: { summary: Summary; refresh: () => void; onError: (e: string) => void }) {
  const t = useT();
  const [rows, setRows] = useState<SpeciesRow[]>([]);
  const [sel, setSel] = useState<{ label: number; model: string } | null>(null);
  const [queue, setQueue] = useState<Detection[]>([]);
  const [i, setI] = useState(0);
  const [filter, setFilter] = useState("");
  const [training, setTraining] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [pane, setPane] = useState<"clips" | "agree">("clips");
  const [unusual, setUnusual] = useState(false);
  const flags = useOnlineFlags();
  usePrefetch();

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
        ? (r.classes.length === 1 ? t("Trained 1 species; {n} detections found.", { n: r.detections.toLocaleString() }) : t("Trained {species} species; {n} detections found.", { species: r.classes.length, n: r.detections.toLocaleString() }))
          + (r.skipped.length ? " " + t("Not enough examples yet for: {list}.", { list: r.skipped.map((s) => `${s[0]} (${s[1]})`).join(", ") }) : "")
        : t("Not enough examples yet: each species needs at least 3 clips marked yes (or named) in recordings processed with Perch."));
      refresh(); load();
    } catch (e) { onError(String(e)); }
    setTraining(false);
  }
  async function share() {
    const out = await save({ title: t("Save the classifier to share"), defaultPath: "wildlife-network-classifier.wnc", filters: [{ name: "Wildlife Network Classifier", extensions: ["wnc"] }] });
    if (out) try { await api.wncExport(out); setNote(t("Saved to {path}.", { path: out })); } catch (e) { onError(String(e)); }
  }
  async function importPack() {
    const f = await open({ title: t("Add a shared classifier"), multiple: false, filters: [{ name: "Wildlife Network Classifier", extensions: ["wnc"] }] });
    if (typeof f === "string") try { const n = await api.wncImport(f); setNote(t("Added. {n} detections found.", { n: n.toLocaleString() })); refresh(); load(); } catch (e) { onError(String(e)); }
  }

  const row = rows.find((r) => sel && r.label === sel.label && r.model === sel.model) ?? null;
  const anyFlag = rows.some((r) => flags[r.label]);
  const shown = rows.filter((r) => (!filter || `${r.common} ${r.scientific}`.toLowerCase().includes(filter.toLowerCase())) && (!unusual || !anyFlag || flags[r.label]));
  const done = queue.filter((d) => d.verdict).length;
  const hasPerch = summary.deployments.some((d) => d.models.includes("perch-2.0"));
  return (
    <section className="review">
      <div className="species-list">
        <div className="wnc-card">
          <b>Wildlife Network Classifier</b>
          <small className="muted block">{summary.wnc ? t("{n} species learned from your reviews", { n: summary.wnc.classes.length }) : t("Learns your local species from your answers")}</small>
          <div className="row tight">
            <button className="small-btn" onClick={train} disabled={training || !hasPerch} title={hasPerch ? "" : t("Needs recordings processed with Perch")}>{training ? t("Training...") : t("Train")}</button>
            {summary.wnc && <button className="small-btn" onClick={share}>{t("Share")}</button>}
            <button className="small-btn" onClick={importPack}>{t("Add shared")}</button>
          </div>
          {note && <small className="block note-sm">{note}</small>}
        </div>
        <SecondReviewer onCompare={() => setPane("agree")} onError={onError} />
        <input className="search" placeholder={t("Search {n} species", { n: rows.length })} value={filter} onChange={(e) => setFilter(e.target.value)} />
        {anyFlag && <label className="wn-filter"><input type="checkbox" checked={unusual} onChange={(e) => setUnusual(e.target.checked)} /> {t("Unusual for the area")}</label>}
        <ul>
          {shown.map((r) => (
            <li key={`${r.label}|${r.model}`} className={sel && r.label === sel.label && r.model === sel.model ? "active" : ""} onClick={() => pick(r.label, r.model)}>
              <div className="sp-name"><b>{r.common || r.scientific}<FlagMark s={flags[r.label]} /></b><i>{r.common ? r.scientific : ""}</i></div>
              <div className="sp-meta">
                <span title={t("Detections kept at the current threshold, of all in range")}>{r.kept.toLocaleString()} / {r.detections.toLocaleString()}</span>
                <span className={`tag model-${r.model}`} title={modelName(r.model)}>{modelTag(r.model)}</span>
                {r.reviewed > 0 ? <span className={r.threshold_source === "fitted" ? "tag ok" : "tag"}>{t("{n} checked", { n: r.reviewed })}</span> : <span className="tag dim">{t("not checked")}</span>}
              </div>
            </li>
          ))}
        </ul>
        {rows.length === 0 && <p className="muted pad">{t("No detections yet. Process some recordings first.")}</p>}
      </div>
      <div className="clips">
        {pane === "agree" ? <Agreement onChanged={load} onError={onError} /> : !row ? (
          <div className="empty">
            <h2>{t("Review")}</h2>
            <p>{t("Pick a species. You hear a few clips from every confidence level, from sure to doubtful. Mark each one. After 8 answers Studio sets that species' threshold so that what is kept is right at least 90% of the time.")}</p>
            <p>{t("Hear something the models got wrong, or a species they do not know? Use {name} on any clip, then {find}. Train the Wildlife Network Classifier and it will look for that species everywhere.", { name: "\u0001", find: "\u0002" }).split(/([\u0001\u0002])/).map((part, k) => part === "\u0001" ? <b key={k}>{t("Name this sound")}</b> : part === "\u0002" ? <b key={k}>{t("Find more like this")}</b> : part)}</p>
            <p className="muted">{t("Keys: Y yes, N no, U unsure, Space play, arrows to move.")}</p>
          </div>
        ) : (
          <>
            <header className="head">
              <div>
                <h2>{row.common || row.scientific} <i className="muted">{row.common ? row.scientific : ""}</i> <span className={`tag model-${row.model}`}>{modelName(row.model)}</span></h2>
                <p className="muted">
                  {row.threshold_source === "fitted"
                    ? row.threshold > 1 ? <>{t("Your answers so far say none of these are reliable: none will be exported unless you mark them yes.")}</>
                      : <>{t("Kept from confidence {score} up ({kept} detections), estimated {pct}% right, from {n} answers.", { score: row.threshold.toFixed(1), kept: row.kept.toLocaleString(), pct: Math.round((row.precision ?? 0) * 100), n: row.reviewed })}</>
                    : <>{t("Threshold {value} (default) until {n} more answers.", { value: row.threshold.toFixed(2), n: Math.max(0, 8 - row.reviewed) })}</>}
                </p>
              </div>
              <button onClick={() => pick(row.label, row.model)}>{done === queue.length && queue.length ? t("Next round") : t("New clips")}</button>
            </header>
            {queue.length === 0 ? <p className="muted">{t("Every detection of this species has been reviewed.")}</p> : (
              <>
                <ClipView key={queue[i].id} d={queue[i]} onVerdict={verdict} onMove={(k) => setI((x) => Math.max(0, Math.min(queue.length - 1, x + k)))} />
                <div className="strip">
                  {queue.map((d, k) => (
                    <button key={d.id} className={`dot ${k === i ? "cur" : ""} ${d.verdict ?? ""}`} onClick={() => setI(k)} title={t("confidence {value}", { value: d.confidence.toFixed(2) })}>{d.confidence.toFixed(2)}</button>
                  ))}
                </div>
                <SpeciesBox d={queue[i]} />
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
  const t = useT();
  const [name, setName] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  useEffect(() => { api.reviewerName().then((n) => setName(n ?? "")).catch(() => {}); }, []);
  async function saveMine() {
    const out = await save({ title: t("Save your review answers"), defaultPath: `${name.trim().replace(/[^\w-]+/g, "-") || "my"}-answers.wlnr`, filters: [{ name: t("Review answers"), extensions: ["wlnr"] }] });
    if (out) try { const n = await api.answersExport(out, name.trim() || null); setMsg(t("{n} answers saved. Send the file to the other reviewer.", { n: n.toLocaleString() })); } catch (e) { onError(String(e)); }
  }
  async function loadOther() {
    const f = await open({ title: t("Load another reviewer's answers"), multiple: false, filters: [{ name: t("Review answers"), extensions: ["wlnr"] }] });
    if (typeof f !== "string") return;
    try {
      const r = await api.answersLoad(f);
      setMsg(t("{matched} of {total} answers from {who} matched detections here.", { matched: r.matched.toLocaleString(), total: r.answers.toLocaleString(), who: r.reviewer }) + (r.unmatched ? " " + t("{n} did not: their copy may have been processed differently.", { n: r.unmatched.toLocaleString() }) : ""));
      onCompare();
    } catch (e) { onError(String(e)); }
  }
  return (
    <details className="wnc-card">
      <summary><b>{t("Second reviewer")}</b></summary>
      <small className="muted block">{t("Two people review the same recordings on their own, then compare. Your own answers are never overwritten.")}</small>
      <input className="search" value={name} onChange={(e) => setName(e.target.value)} placeholder={t("Your name (optional)")} aria-label={t("Your name")} />
      <div className="row tight">
        <button className="small-btn" onClick={saveMine}>{t("Save my answers")}</button>
        <button className="small-btn" onClick={loadOther}>{t("Load other answers")}</button>
        <button className="small-btn" onClick={onCompare}>{t("Compare")}</button>
      </div>
      {msg && <small className="block note-sm">{msg}</small>}
    </details>
  );
}

/** "80% (49 to 94)": the value with its 95% interval, when there is one. */
const withCi = (v: number | null, lo: number | null, hi: number | null, f: (x: number) => string) =>
  v == null ? "" : lo == null || hi == null ? f(v) : `${f(v)} (${f(lo)} ${tr("to")} ${f(hi)})`;
/** A stored answer (yes, no, unsure, nothing) in the interface language. */
const VERDICTS: Record<string, string> = { yes: "yes", no: "no", unsure: "unsure", nothing: "nothing" };
const verdictWord = (v: string | null) => (v && VERDICTS[v] ? tr(VERDICTS[v]) : v ?? "");

/** Agreement per species with the other reviewer, and the disagreements to settle one by one. */
function Agreement({ onChanged, onError }: { onChanged: () => void; onError: (e: string) => void }) {
  const t = useT();
  const [rep, setRep] = useState<AgreementReport | null>(null);
  const [cur, setCur] = useState<number | null>(null);
  const load = useCallback(async (who: string | null) => { try { setRep(await api.agreementReport(who)); } catch (e) { onError(String(e)); } }, [onError]);
  useEffect(() => { load(null); }, [load]);
  const list = rep?.disagreements ?? [];
  const [tableNote, setTableNote] = useState<string | null>(null);
  async function saveTable() {
    const out = await save({ title: t("Save table for a paper (CSV)"), defaultPath: t("agreement-and-precision") + ".csv", filters: [{ name: "CSV", extensions: ["csv"] }] });
    if (!out) return;
    try { const n = await api.agreementTable(rep?.reviewer ?? null, out); setTableNote(t("Saved {n} species.", { n })); } catch (e) { onError(String(e)); }
  }
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
  if (!rep) return <p className="muted">{t("Loading...")}</p>;
  if (!rep.reviewer) return (
    <div className="empty"><h2>{t("Second reviewer")}</h2>
      <p>{t("Ask a colleague to open a copy of this project, review on their own, and use {save}. Load their file with {load} to see where you agree, species by species.", { save: "\u0001", load: "\u0002" }).split(/([\u0001\u0002])/).map((part, k) => part === "\u0001" ? <b key={k}>{t("Save my answers")}</b> : part === "\u0002" ? <b key={k}>{t("Load other answers")}</b> : part)}</p></div>
  );
  const who = rep.reviewer;
  const sp = (x: { common: string; scientific: string }) => x.common || x.scientific;
  return (
    <>
      <header className="head">
        <div>
          <h2>{t("Agreement with {who}", { who })}</h2>
          <p className="muted">{t("Detections you both reviewed. Agreement and kappa count yes and no answers only (not sure is left out). Kappa corrects for agreeing by chance: 1 is full agreement, 0 is no better than chance. Settling a disagreement sets the final answer but does not change these figures.")}</p>
        </div>
        {rep.reviewers.length > 1 && <select value={who} onChange={(e) => { setCur(null); load(e.target.value); }} aria-label={t("Reviewer")}>{rep.reviewers.map((r) => <option key={r}>{r}</option>)}</select>}
      </header>
      {d ? (
        <div className="card">
          <p><b>{sp(d)}</b> <span className={`tag model-${d.model}`}>{modelTag(d.model)}</span> {t("You said {mine}, {who} said {theirs}. Listen again and choose the final answer. ({n} of {total})", { mine: "\u0001", who, theirs: "\u0002", n: (cur ?? 0) + 1, total: list.length }).split(/([\u0001\u0002])/).map((part, k) => part === "\u0001" ? <b key={k}>{verdictWord(d.verdict ?? "nothing")}</b> : part === "\u0002" ? <b key={k}>{verdictWord(d.other)}</b> : part)}</p>
          <ClipView key={d.id} d={d} onVerdict={settle} onMove={move} />
          <div className="row tight"><button onClick={() => setCur(null)}>{t("Back to the table")}</button></div>
        </div>
      ) : (
        <>
          <table className="grid">
            <thead><tr><th>{t("Species")}</th><th title={t("Detections you both reviewed")}>{t("Both reviewed")}</th><th title={t("Both answered yes or no")}>{t("Yes or no")}</th><th>{t("Agree")}</th><th>{t("Kappa")}</th></tr></thead>
            <tbody>
              {[rep.overall, ...rep.species].map((x, k) => (
                <tr key={x.scientific} className={k === 0 ? "total" : ""}>
                  <td>{k === 0 ? <b>{t("All species")}</b> : <>{sp(x)} {x.common && <i className="muted">{x.scientific}</i>}</>}</td>
                  <td>{x.both}</td><td title={t("Both yes {yy}, both no {nn}, you yes and {who} no {yn}, you no and {who} yes {ny}", { yy: x.yy, nn: x.nn, yn: x.yn, ny: x.ny, who })}>{x.yes_no}</td>
                  <td>{withCi(x.percent, x.percent_low, x.percent_high, (v) => `${Math.round(v)}%`)}</td>
                  <td title={x.kappa == null && x.yes_no ? t("Not defined: you both gave the same answer every time") : ""}>{x.kappa == null ? (x.yes_no ? t("n/a") : "") : withCi(x.kappa, x.kappa_low, x.kappa_high, (v) => v.toFixed(2))}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="muted small">{t("In brackets: 95% interval (Wilson for agreement, large-sample for kappa). With few clips the interval is wide.")}</p>
          <div className="row tight">
            <button onClick={saveTable}>{t("Save table for a paper (CSV)")}</button>
            {tableNote && <span className="small">{tableNote}</span>}
          </div>
          <details className="small"><summary>{t("Methods sentence")}</summary>
            <p className="muted">{t("Two reviewers independently checked {n} detections in Wildlife Network Studio, answering yes, no or not sure. Agreement was measured on yes and no answers with Cohen's kappa (overall {kappa}); disagreements were then settled together. Precision per species is the share of reviewed detections at or above the species' threshold answered yes, with Wilson 95% intervals.", { n: rep.overall.yes_no, kappa: withCi(rep.overall.kappa, rep.overall.kappa_low, rep.overall.kappa_high, (v) => v.toFixed(2)) || t("n/a") })}</p>
          </details>
          <div className="card disagree">
            <h3>{t("Disagreements to settle ({n})", { n: list.length })}{rep.settled ? <small className="muted"> {t("{n} settled", { n: rep.settled })}</small> : null}</h3>
            {list.length === 0 ? <p className="muted">{t("None left.")}</p> : (
              <>
                <div className="row tight"><button className="primary" onClick={() => setCur(0)}>{t("Settle them one by one")}</button></div>
                {list.map((x, k) => (
                  <div key={x.id} className="sim-row">
                    <button className="small-btn" onClick={() => setCur(k)}>{t("Open")}</button>
                    <span className="small"><b>{sp(x)}</b> {modelTag(x.model)}, {t("{file} at {sec} s", { file: x.path.split(/[\\/]/).pop() ?? "", sec: x.start_s.toFixed(0) })}</span>
                    <span className="small">{t("You: {mine}, {who}: {theirs}", { mine: verdictWord(x.verdict), who, theirs: verdictWord(x.other) })}</span>
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
  const t = useT();
  const [name, setName] = useState("");
  const [common, setCommon] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const [similar, setSimilar] = useState<Similar[] | null>(null);
  const [marked, setMarked] = useState<Record<number, boolean>>({});
  const sci = name.trim() || d.scientific;
  async function nameIt(present: boolean) {
    try {
      const ok = await api.nameSound(d.id, sci, common.trim() || (name.trim() ? "" : d.common), present);
      setMsg(ok ? (present ? t("Saved: this clip is {name}.", { name: sci }) : t("Saved: {name} is not in this clip.", { name: sci })) : t("This recording has not been through Perch yet, so it cannot teach the classifier. Run Perch on it first."));
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
      <summary>{t("Name this sound, or find more like it")}</summary>
      <p className="muted small">{t("For the Wildlife Network Classifier. Use the scientific name; it can be a species BirdNET and Perch do not know.")}</p>
      <div className="row">
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder={d.scientific} aria-label={t("Scientific name")} />
        <input value={common} onChange={(e) => setCommon(e.target.value)} placeholder={t("Common name (optional)")} aria-label={t("Common name")} />
      </div>
      <div className="row tight">
        <button onClick={() => nameIt(true)}>{t("This is {name}", { name: sci })}</button>
        <button onClick={() => nameIt(false)}>{t("{name} is not here", { name: sci })}</button>
        {/* Imported results have no Perch embeddings to search. */}
        {!isImported(d.model) && <button onClick={findMore}>{t("Find more like this")}</button>}
      </div>
      {msg && <p className="note">{msg}</p>}
      {similar && (
        <div className="similar">
          <p className="muted small">{t("The most similar 5 s windows in this project. Listen and mark the ones that are {name}; then train the classifier.", { name: sci })}</p>
          {similar.length === 0 && <p className="muted">{t("Nothing similar found.")}</p>}
          {similar.map((w) => <SimilarRow key={w.window_id} w={w} sci={sci} mark={marked[w.window_id]} onMark={(p) => mark(w, p)} onError={onError} />)}
        </div>
      )}
    </details>
  );
}

function SimilarRow({ w, sci, mark, onMark, onError }: { w: Similar; sci: string; mark: boolean | undefined; onMark: (p: boolean) => void; onError: (e: string) => void }) {
  const t = useT();
  const a = useRef<HTMLAudioElement | null>(null);
  async function play() {
    try {
      if (!a.current) { const c = await api.clip(w.path, w.start_s, w.end_s); a.current = new Audio(`data:audio/wav;base64,${c.wav}`); }
      a.current.currentTime = 0; a.current.play();
    } catch (e) { onError(String(e)); }
  }
  return (
    <div className="sim-row">
      <button className="small-btn" onClick={play}>{t("Play")}</button>
      <span className="mono small">{Math.round(w.similarity * 100)}%</span>
      <span className="small">{w.site}, {t("{file} at {sec} s", { file: w.path.split(/[\\/]/).pop() ?? "", sec: w.start_s.toFixed(0) })}</span>
      <span className="row tight">
        <button className={`small-btn ${mark === true ? "on-yes" : ""}`} onClick={() => onMark(true)}>{sci}</button>
        <button className={`small-btn ${mark === false ? "on-no" : ""}`} onClick={() => onMark(false)}>{t("Not it")}</button>
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
  const t = useT();
  const canvas = useRef<HTMLCanvasElement>(null);
  const audio = useRef<HTMLAudioElement | null>(null);
  const [clip, setClip] = useState<Clip | null>(null);
  const [playing, setPlaying] = useState(false);
  const [pos, setT] = useState(0);
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
        {clip ? <canvas ref={canvas} /> : <div className="loading">{failed ? t("This clip could not be read: {error}", { error: failed }) : t("Loading clip...")}</div>}
        {clip && <div className="window" style={box} />}
        {clip && <div className="cursor" style={{ left: `${(pos / total) * 100}%` }} />}
        {clip && <span className="khz">{clip.max_khz} kHz</span>}
        {clip && clip.slowed > 1 && <span className="slowed">{t("played {x}x slower", { x: Math.round(clip.slowed) })}</span>}
      </div>
      <div className="clip-meta">
        {d.detection_score != null
          ? <><span>{t("Species")} <b>{d.confidence.toFixed(2)}</b></span><span>{t("Bat call")} <b>{d.detection_score.toFixed(2)}</b></span></>
          : <span>{t("Confidence")} <b>{d.confidence.toFixed(2)}</b></span>}
        <span>{d.site}</span>
        <span>{d.utc ? new Date(d.utc + "Z").toLocaleString() : t("time unknown")}</span>
        <span className="muted mono" title={d.path}>{t("{file} at {sec} s", { file: d.path.split(/[\\/]/).pop() ?? "", sec: d.start_s.toFixed(0) })}</span>
      </div>
      <div className="verdicts">
        <button onClick={toggle}>{playing ? t("Pause") : t("Play")} <kbd>Space</kbd></button>
        <button className={`yes ${d.verdict === "yes" ? "on" : ""}`} onClick={() => onVerdict("yes")}>{t("Yes, it is")} <kbd>Y</kbd></button>
        <button className={`no ${d.verdict === "no" ? "on" : ""}`} onClick={() => onVerdict("no")}>{t("No")} <kbd>N</kbd></button>
        <button className={`unsure ${d.verdict === "unsure" ? "on" : ""}`} onClick={() => onVerdict("unsure")}>{t("Not sure")} <kbd>U</kbd></button>
      </div>
    </div>
  );
}
