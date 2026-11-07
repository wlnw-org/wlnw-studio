import { api, fmtBytes, fmtDuration, fmtHours, modelName, type Progress, type Summary } from "../api";
import ModelPacks from "../ModelPacks";

/** Choose the models, then start, pause and resume. Each recording is saved per model when finished, so stopping or a
 * crash loses at most one file. */
export default function Process({ summary, live, refresh, onError, next }: { summary: Summary; live: Progress | null; refresh: () => void; onError: (e: string) => void; next: () => void }) {
  const p = live && summary.running ? live : summary.progress;
  const pct = p.bytes_total ? Math.min(100, (p.bytes_done / p.bytes_total) * 100) : 0;
  const left = p.jobs_total - p.jobs_done;
  const auto = summary.chosen.length === 0;
  async function choose(ids: string[]) { try { await api.setSetting("models", JSON.stringify(ids)); refresh(); } catch (e) { onError(String(e)); } }
  function toggle(id: string) {
    const cur = auto ? [] : summary.chosen;
    const nextIds = cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id];
    choose(nextIds);
  }
  async function start() { try { await api.startRun(); refresh(); } catch (e) { onError(String(e)); } }
  return (
    <section>
      <header className="head">
        <div>
          <h2>Find species</h2>
          <p className="muted">Everything runs on this computer: nothing is uploaded. Pause any time; Studio carries on where it stopped.</p>
        </div>
        {summary.running
          ? <button onClick={() => api.stopRun()}>Pause after this file</button>
          : <button className="primary" onClick={start} disabled={summary.models.length === 0 || left <= 0}>{p.jobs_done > 0 && left > 0 ? "Resume" : "Start"}</button>}
      </header>
      {summary.model_errors.map((e) => <p key={e} className="note">{e}</p>)}
      <ModelPacks onChange={refresh} onError={onError} />

      {summary.models.length > 0 && <div className="card">
        <h3>Which models to run</h3>
        <label className="choice"><input type="radio" checked={auto} onChange={() => choose([])} disabled={summary.running} />
          <span><b>Best for each recorder&apos;s region</b> (recommended)<small>Perch first in Africa and South Asia, where BirdNET knows fewer species, with BirdNET as a second opinion. BirdNET elsewhere: it is faster and filters species by place and week.</small></span></label>
        <label className="choice"><input type="radio" checked={!auto} onChange={() => choose(summary.models.map((m) => m.id))} disabled={summary.running} />
          <span><b>Choose myself</b><small>The same models for every recorder.</small></span></label>
        <div className="models">
          {summary.models.map((m) => (
            <label key={m.id} className={`model ${auto ? "dim" : ""}`}>
              <input type="checkbox" disabled={auto || summary.running} checked={auto ? false : summary.chosen.includes(m.id)} onChange={() => toggle(m.id)} />
              <span><b>{m.name} {m.version}</b>
                <small>{m.id === "perch-2.0" ? "About 15,000 species of birds, frogs, insects and mammals. Needed for the Wildlife Network Classifier." : "About 6,500 species, mostly birds, filtered by place and week."}
                  {" "}About {Math.round(m.speed_x)}x real time on a recent laptop. Licence: <a href={m.licence_url} target="_blank" rel="noreferrer">{m.licence}</a>.</small></span>
            </label>
          ))}
        </div>
      </div>}

      <div className="card">
        <div className="bar"><div style={{ width: `${pct}%` }} /></div>
        <div className="stats">
          <div><b>{p.files_done.toLocaleString()}</b> of {p.files_total.toLocaleString()} recordings done</div>
          <div><b>{fmtHours(p.hours_done)}</b> of audio</div>
          <div><b>{p.detections.toLocaleString()}</b> detections</div>
          {summary.running && live && live.speed_x > 0 && <div><b>{Math.round(live.speed_x)}x</b> real time</div>}
          {summary.running && live?.eta_s != null && live.eta_s > 0 && <div>about <b>{fmtDuration(live.eta_s)}</b> left</div>}
          <div className="muted">{fmtBytes(p.bytes_done)} of {fmtBytes(p.bytes_total)}</div>
        </div>
        {summary.running && live?.current && <p className="muted small mono">{live.current_model}: {live.current}</p>}
        {p.errors > 0 && <p className="warn">{p.errors} {p.errors === 1 ? "file" : "files"} could not be read (damaged or not audio). They are skipped.</p>}
        {!summary.running && summary.deployments.some((d) => d.models.length === 0) && <p className="warn">Some recorders have no model to run.</p>}
        <p className="muted small">Per recorder: {summary.deployments.map((d) => `${d.name}: ${d.models.map(modelName).join(" + ")}`).join("; ")}</p>
      </div>
      {p.jobs_done > 0 && <div className="foot"><button className="primary" onClick={next}>Next: review</button></div>}
    </section>
  );
}
