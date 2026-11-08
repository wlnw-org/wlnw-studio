import { api, fmtBytes, fmtDuration, fmtHours, modelName, type Progress, type Summary } from "../api";
import ModelPacks from "../ModelPacks";
import { useT } from "../i18n";

/** Choose the models, then start, pause and resume. Each recording is saved per model when finished, so stopping or a
 * crash loses at most one file. */
export default function Process({ summary, live, refresh, onError, next, showFiles }: { summary: Summary; live: Progress | null; refresh: () => void; onError: (e: string) => void; next: () => void; showFiles: (status?: string) => void }) {
  const t = useT();
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
          <h2>{t("Find species")}</h2>
          <p className="muted">{t("Everything runs on this computer: nothing is uploaded. Pause any time; Studio carries on where it stopped.")}</p>
        </div>
        {summary.running
          ? <button onClick={() => api.stopRun()}>{t("Pause after this file")}</button>
          : <button className="primary" onClick={start} disabled={summary.models.length === 0 || left <= 0}>{p.jobs_done > 0 && left > 0 ? t("Resume") : t("Start")}</button>}
      </header>
      {summary.model_errors.map((e) => <p key={e} className="note">{e}</p>)}
      {!summary.space.enough && left > 0 && <p className="warn card">{t("Perch saves a fingerprint of every 5 seconds of audio next to the project, for \"find more like this\" and the Wildlife Network Classifier. The recordings still to process need about {needed}, but the disk with the project has only {free} free. Free some space or save the project on a bigger disk (it can sit apart from the recordings). Processing stops by itself if less than 1 GB is left.", { needed: fmtBytes(summary.space.needed), free: fmtBytes(summary.space.free ?? 0) })}</p>}
      <ModelPacks onChange={refresh} onError={onError} />

      {summary.images && !summary.models.some((m) => m.id === "speciesnet-4.0.3a") && <p className="note">{t("This project has {n} camera-trap images. Add the SpeciesNet pack above to find the animals in them.", { n: summary.images.total.toLocaleString() })}</p>}
      {summary.images && summary.models.some((m) => m.id === "speciesnet-4.0.3a") && <p className="muted">{t("Camera-trap images always go through SpeciesNet: MegaDetector finds animals, people and vehicles, then SpeciesNet names the animal in each box, checked against the camera's country (set it on the Recordings screen).")}</p>}
      {summary.models.some((m) => m.id !== "speciesnet-4.0.3a") && <div className="card">
        <h3>{t("Which models to run")}</h3>
        <label className="choice"><input type="radio" checked={auto} onChange={() => choose([])} disabled={summary.running} />
          <span><b>{t("Best for each recorder's region")}</b> {t("(recommended)")}<small>{t("Perch first in Africa and South Asia, where BirdNET knows fewer species, with BirdNET as a second opinion. BirdNET elsewhere: it is faster and filters species by place and week. Bat recordings (192 kHz and above) go to BatDetect2 when it is installed.")}</small></span></label>
        <label className="choice"><input type="radio" checked={!auto} onChange={() => choose(summary.models.map((m) => m.id))} disabled={summary.running} />
          <span><b>{t("Choose myself")}</b><small>{t("The same models for every recorder. BatDetect2 only runs on bat recordings (192 kHz and above); bird models run on every recorder you tick them for, bat recorders included.")}</small></span></label>
        <div className="models">
          {summary.models.filter((m) => m.id !== "speciesnet-4.0.3a").map((m) => (
            <label key={m.id} className={`model ${auto ? "dim" : ""}`}>
              <input type="checkbox" disabled={auto || summary.running} checked={auto ? false : summary.chosen.includes(m.id)} onChange={() => toggle(m.id)} />
              <span><b>{m.name} {m.version}</b>
                <small>{m.id === "perch-2.0" ? t("About 15,000 species of birds, frogs, insects and mammals. Needed for the Wildlife Network Classifier.")
                  : m.id === "batdetect2-uk" ? t("17 UK bat species, call by call, in ultrasonic recordings (192 kHz and above, or time expanded). Non-commercial use only.")
                  : t("About 6,500 species, mostly birds, filtered by place and week.")}
                  {" "}{t("About {x}x real time on a recent laptop.", { x: Math.round(m.speed_x) })} {t("Licence:")} <a href={m.licence_url} target="_blank" rel="noreferrer">{m.licence}</a>.</small></span>
            </label>
          ))}
        </div>
      </div>}

      <div className="card">
        <div className="bar"><div style={{ width: `${pct}%` }} /></div>
        <div className="stats">
          {(p.files_total > 0 || !p.images_total) && <div><b>{p.files_done.toLocaleString()}</b> {t("of {total} recordings done", { total: p.files_total.toLocaleString() })}</div>}
          {p.images_total > 0 && <div><b>{p.images_done.toLocaleString()}</b> {t("of {total} images done", { total: p.images_total.toLocaleString() })}</div>}
          {summary.running && live && live.images_per_s > 0 && <div><b>{live.images_per_s.toFixed(1)}</b> {t("images a second")}</div>}
          <div><b>{fmtHours(p.hours_done)}</b> {t("of audio")}</div>
          <div><b>{p.detections.toLocaleString()}</b> {t("detections")}</div>
          {summary.running && live && live.speed_x > 0 && <div><b>{Math.round(live.speed_x)}x</b> {t("real time")}</div>}
          {summary.running && live?.eta_s != null && live.eta_s > 0 && <div>{t("about")} <b>{fmtDuration(live.eta_s)}</b> {t("left")}</div>}
          <div className="muted">{t("{done} of {total}", { done: fmtBytes(p.bytes_done), total: fmtBytes(p.bytes_total) })}</div>
          {summary.space.enough && summary.space.needed > 1e9 && left > 0 && <div className="muted">{summary.space.free != null ? t("Needs about {needed} of disk space, {free} free", { needed: fmtBytes(summary.space.needed), free: fmtBytes(summary.space.free) }) : t("Needs about {needed} of disk space", { needed: fmtBytes(summary.space.needed) })}</div>}
        </div>
        {summary.running && live?.current && <p className="muted small mono">{live.current_model}: {live.current}</p>}
        {p.errors > 0 && <p className="warn">{p.errors === 1 ? t("1 file could not be read (damaged or not audio). It is skipped.") : t("{n} files could not be read (damaged or not audio). They are skipped.", { n: p.errors })} <button className="link inline" onClick={() => showFiles("failed")}>{t("See which and why")}</button></p>}
        {!summary.running && summary.deployments.some((d) => d.models.length === 0) && <p className="warn">{t("Some recorders have no model to run.")}</p>}
        <p className="small"><button className="link" onClick={() => showFiles()}>{t("See every recording and its status")}</button></p>
        <p className="muted small">{t("Per recorder: {list}", { list: summary.deployments.map((d) => `${d.name}: ${[...(d.files > 0 ? d.models.map(modelName) : []), ...(d.images > 0 ? ["SpeciesNet"] : [])].join(" + ") || t("none")}`).join("; ") })}</p>
      </div>
      {p.jobs_done > 0 && <div className="foot"><button className="primary" onClick={next}>{t("Next: review")}</button></div>}
    </section>
  );
}
