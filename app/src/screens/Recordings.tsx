import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, fmtHours, isLive, modelName, modelTag, type Deployment, type LiveReport, type LiveStation, type Summary } from "../api";
import MapPicker from "../MapPicker";
import { AroundRecorders, usePrefetch } from "../Online";
import { t as tr, useT, type Vars } from "../i18n";

/** Picks the one or many form of an English template and fills {n} with the formatted number. */
const count = (n: number, one: string, many: string, vars: Vars = {}) => tr(n === 1 ? one : many, { n: n.toLocaleString(), ...vars });

/** Add recordings, then confirm each recorder's place and time zone (filled in from the files when they carry it). */
export default function Recordings({ summary, refresh, onError, next }: { summary: Summary; refresh: () => void; onError: (e: string) => void; next: () => void }) {
  const t = useT();
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<string | null>(null);
  usePrefetch();
  async function add() {
    const folder = await open({ title: t("Choose a folder or drive with recordings"), directory: true, multiple: false });
    if (typeof folder !== "string") return;
    setBusy(true); setReport(null);
    try {
      const r = await api.scanFolder(folder);
      const where = count(r.deployments, "{n} recorder or camera", "{n} recorders and cameras");
      setReport((r.images_found
        ? t("{found} recordings found, {added} new, and {images} camera-trap images, {imagesNew} new, in {where}.", { found: r.found.toLocaleString(), added: r.added.toLocaleString(), images: r.images_found.toLocaleString(), imagesNew: r.images_added.toLocaleString(), where })
        : t("{found} recordings found, {added} new, in {where}.", { found: r.found.toLocaleString(), added: r.added.toLocaleString(), where })) +
        (r.without_time ? " " + count(r.without_time, "{n} recording has no date in their name or metadata and will be skipped for publishing.", "{n} recordings have no date in their name or metadata and will be skipped for publishing.") : "") +
        (r.images_without_time ? " " + count(r.images_without_time, "{n} image has no date in their EXIF data and will be skipped for publishing.", "{n} images have no date in their EXIF data and will be skipped for publishing.") : ""));
      refresh();
    } catch (e) { onError(String(e)); }
    setBusy(false);
  }
  async function importResults(folder: boolean) {
    const f = await open(folder ? { title: t("Choose a folder of results tables"), directory: true, multiple: false }
      : { title: t("Choose a results table"), multiple: false, filters: [{ name: t("Results table"), extensions: ["csv", "txt"] }] });
    if (typeof f !== "string") return;
    setBusy(true); setReport(null);
    try {
      const r = await api.importResults(f);
      const names = r.unknown_names.slice(0, 5).join(", ");
      setReport(t("{added} new detections from {models}: {matched} of {rows} rows matched to recordings in this project.", { added: r.added.toLocaleString(), models: r.model.split(", ").map(modelTag).join(` ${t("and")} `), matched: r.matched.toLocaleString(), rows: r.rows.toLocaleString() }) +
        (r.unmatched ? " " + count(r.unmatched, "{n} row did not match a recording here (for example {examples}). Add those recordings first, then import again: nothing is added twice.", "{n} rows did not match a recording here (for example {examples}). Add those recordings first, then import again: nothing is added twice.", { examples: r.unmatched_examples.slice(0, 3).join(", ") }) : "") +
        (r.no_species ? " " + (r.unknown_names.length
          ? count(r.no_species, "{n} row had no species (noise or no ID), or a name Studio did not know: {names}.", "{n} rows had no species (noise or no ID), or a name Studio did not know: {names}.", { names })
          : count(r.no_species, "{n} row had no species (noise or no ID).", "{n} rows had no species (noise or no ID).")) : "") +
        (r.added ? " " + t("Review them on the Review screen.") : ""));
      refresh();
    } catch (e) { onError(String(e)); }
    setBusy(false);
  }
  const deps = summary.deployments;
  const missing = deps.filter((d) => d.lat == null || !d.tz_known).length;
  const cams = deps.some((d) => d.images > 0);
  const hours = deps.reduce((s, d) => s + d.hours, 0);
  return (
    <section>
      <header className="head">
        <div>
          <h2>{t("Recordings")}</h2>
          <p className="muted">{t("Add a whole drive or SD card copy. Subfolders are searched; WAV, FLAC and MP3 recordings and JPEG and PNG camera-trap images are read. Files already added are skipped.")}</p>
        </div>
        <button className="primary" onClick={add} disabled={busy}>{busy ? t("Reading files...") : t("Add a folder or drive")}</button>
      </header>
      {report && <p className="note">{report}</p>}
      {deps.length === 0 ? (
        <>
          <div className="empty">{t("No recordings yet. Studio reads the time and place written by AudioMoth and Song Meter recorders, and the date in file names such as {example}.", { example: "SMM01234_20250601_053000.wav" })}</div>
          <Live refresh={refresh} onError={onError} setReport={setReport} />
        </>
      ) : (
        <>
          <p className="muted">{[count(deps.length, "{n} recorder", "{n} recorders"), t("{n} files", { n: deps.reduce((s, d) => s + d.files, 0).toLocaleString() }),
              ...(cams ? [t("{n} images", { n: deps.reduce((s, d) => s + d.images, 0).toLocaleString() })] : []),
              ...(hours > 0 ? [t("{hours} processed", { hours: fmtHours(hours) })] : [])].join(", ")}.
            {missing > 0 && <b className="warn"> {t("{n} need a place or time zone: the place decides which species are expected and which models run.", { n: missing })}</b>}</p>
          <table className="grid">
            <thead><tr><th>{t("Recorder")}</th><th>{t("Files")}</th><th>{t("Place")}</th><th>{t("Times are UTC +")}</th>{cams && <th title={t("Three-letter code (ISO 3166). SpeciesNet uses it to rule out species that do not live there.")}>{t("Country")}</th>}<th>{t("Models")}</th><th /></tr></thead>
            <tbody>{deps.map((d) => <Row key={d.id} d={d} cams={cams} onSaved={refresh} onError={onError} />)}</tbody>
          </table>
          {cams && <p className="muted small">{t("Camera traps: set each camera's country (three letters, for example KEN, IND, BRA or USA, and the state code for the USA) so SpeciesNet can rule out species that do not live there. Changing it runs that camera's images again.")}</p>}
          <AroundRecorders deps={deps} />
          <div className="card import-card">
            <h3>{t("Import results from another tool")}</h3>
            <p className="muted">{t("Already ran BirdNET-Analyzer, BirdNET-Pi, BirdNET-Go or Kaleidoscope on these recordings? Import their results to review and publish them here without processing the audio again.")}
              {" "}{t("Studio reads BirdNET-Analyzer CSV and Raven selection tables, BirdNET-Pi and BirdNET-Go detection exports, and Kaleidoscope id.csv. Each row is matched to a recording added above by its path or file name (BirdNET-Pi and Go rows by date and time).")}
              {" "}{t("Kaleidoscope manual IDs count as review answers.")}</p>
            <div className="row tight">
              <button onClick={() => importResults(false)} disabled={busy}>{t("Import a results file")}</button>
              <button onClick={() => importResults(true)} disabled={busy}>{t("Import a folder of results")}</button>
            </div>
          </div>
          <Live refresh={refresh} onError={onError} setReport={setReport} />
          <div className="foot"><button className="primary" onClick={next}>{t("Next: find species")}</button></div>
        </>
      )}
    </section>
  );
}

function Row({ d, cams, onSaved, onError }: { d: Deployment; cams: boolean; onSaved: () => void; onError: (e: string) => void }) {
  const t = useT();
  const [country, setCountry] = useState(d.country ?? "");
  const [state, setState] = useState(d.admin1 ?? "");
  const countryDirty = country.trim().toUpperCase() !== (d.country ?? "") || (country.trim().toUpperCase() === "USA" && state.trim().toUpperCase() !== (d.admin1 ?? ""));
  async function saveCountry() {
    try { await api.setDeploymentCountry(d.id, country.trim() || null, state.trim() || null); onSaved(); } catch (e) { onError(String(e)); }
  }
  const [v, setV] = useState({ name: d.name, lat: d.lat?.toString() ?? "", lon: d.lon?.toString() ?? "", tz: (d.tz_offset_min / 60).toString() });
  const [saved, setSaved] = useState(false);
  const [map, setMap] = useState(false);
  const dirty = v.name !== d.name || v.lat !== (d.lat?.toString() ?? "") || v.lon !== (d.lon?.toString() ?? "") || Number(v.tz) * 60 !== d.tz_offset_min || !d.tz_known;
  async function save(patch: Partial<typeof v> = {}) {
    const x = { ...v, ...patch };
    const num = (s: string) => (s.trim() === "" ? null : Number(s));
    try { await api.updateDeployment(d.id, x.name.trim() || d.name, num(x.lat), num(x.lon), Number(x.tz) || 0); setSaved(true); onSaved(); } catch (e) { onError(String(e)); }
  }
  return (
    <tr>
      <td><input value={v.name} onChange={(e) => setV({ ...v, name: e.target.value })} aria-label={t("Recorder name")} /><small className="muted block">{d.model ?? ""} {d.folder ? d.folder.split(/[\\/]/).slice(-2).join("/") : d.serial}</small></td>
      <td>{d.files}{d.images > 0 && <small className="muted block">{t("{n} images", { n: d.images.toLocaleString() })}</small>}</td>
      <td>
        <div className="place">
          <input className={d.lat == null ? "need" : ""} value={v.lat} onChange={(e) => setV({ ...v, lat: e.target.value })} placeholder={t("Latitude")} inputMode="decimal" aria-label={t("Latitude")} />
          <input className={d.lon == null ? "need" : ""} value={v.lon} onChange={(e) => setV({ ...v, lon: e.target.value })} placeholder={t("Longitude")} inputMode="decimal" aria-label={t("Longitude")} />
          <button className="small-btn" onClick={() => setMap(true)} title={t("Choose on a map")}>{t("Map")}</button>
        </div>
      </td>
      <td><input className={!d.tz_known ? "need tz" : "tz"} value={v.tz} onChange={(e) => setV({ ...v, tz: e.target.value })} inputMode="decimal" aria-label={t("UTC offset in hours")} title={d.tz_known ? t("From the recording or confirmed") : t("Please confirm: Song Meters usually record local time, AudioMoths UTC")} /></td>
      {cams && <td>{d.images > 0 && <>
        <input className={`country ${d.country ? "" : "need"}`} value={country} maxLength={3} onChange={(e) => setCountry(e.target.value)} placeholder="KEN" aria-label={t("Country code")} />
        {country.trim().toUpperCase() === "USA" && <input className="country" value={state} maxLength={2} onChange={(e) => setState(e.target.value)} placeholder={t("State")} aria-label={t("US state code")} />}
        {countryDirty && <button className="small-btn" onClick={saveCountry}>{t("Save")}</button>}
      </>}</td>}
      <td className="small">{isLive(d) ? t("Live station: detections from the station") : [...(d.files > 0 ? d.models.map(modelName) : []), ...(d.images > 0 ? ["SpeciesNet"] : [])].join(" + ") || t("none")}{d.perch_region && d.files > 0 && <small className="muted block">{t("Perch first in this region")}</small>}</td>
      <td>{dirty ? <button onClick={() => save()}>{t("Save")}</button> : saved ? <span className="ok">{t("Saved")}</span> : null}</td>
      {map && <MapPicker lat={d.lat} lon={d.lon} title={v.name} onClose={() => setMap(false)}
        onPick={(la, lo) => { const patch = { lat: String(la), lon: String(lo) }; setV({ ...v, ...patch }); setMap(false); save(patch); }} />}
    </tr>
  );
}

const RECENT = "wlnw.liveStations";
const ymd = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
function recentAddresses(): string[] {
  try { const v = JSON.parse(localStorage.getItem(RECENT) ?? "[]"); return Array.isArray(v) ? v.filter((x) => typeof x === "string") : []; } catch { return []; }
}
function remember(address: string) {
  try { localStorage.setItem(RECENT, JSON.stringify([address, ...recentAddresses().filter((a) => a !== address)].slice(0, 8))); } catch { /* storage unavailable */ }
}

/** Detections from a Wildlife Network Base or BirdNET-Go station on the same network, for review like other imported results. */
function Live({ refresh, onError, setReport }: { refresh: () => void; onError: (e: string) => void; setReport: (r: string | null) => void }) {
  const t = useT();
  const [stations, setStations] = useState<LiveStation[]>([]);
  const [address, setAddress] = useState("");
  const [from, setFrom] = useState(() => ymd(new Date(Date.now() - 6 * 86400000)));
  const [to, setTo] = useState(() => ymd(new Date()));
  const [busy, setBusy] = useState<string | null>(null);
  const load = () => api.liveStations().then(setStations).catch(() => {});
  useEffect(() => { load(); }, []);
  const recent = [...new Set([...stations.map((s) => s.address.replace(/^http:\/\//, "")), ...recentAddresses()])];
  function said(r: LiveReport) {
    setReport(count(r.added, "{n} new detection from {station} ({from} to {to}).", "{n} new detections from {station} ({from} to {to}).", { station: r.station, from: r.from, to: r.to }) +
      (r.already ? " " + count(r.already, "{n} was already in the project.", "{n} were already in the project.") : "") +
      (r.no_clip ? " " + count(r.no_clip, "{n} had no clip to listen to and was left out.", "{n} had no clip to listen to and were left out.") : "") +
      (r.no_species ? " " + count(r.no_species, "{n} was not a species (noise, people, engines) and was left out.", "{n} were not a species (noise, people, engines) and were left out.") : "") +
      (r.added ? " " + t("Review them on the Review screen. Each clip is fetched from the station the first time you play it.") : ""));
  }
  async function run(key: string, job: () => Promise<LiveReport>) {
    setBusy(key); setReport(null);
    try { said(await job()); refresh(); load(); } catch (e) { onError(t(String(e))); }
    setBusy(null);
  }
  const add = () => { const a = address.trim(); if (!a) return; remember(a); run("new", () => api.liveImport(a, from, to)); };
  return (
    <div className="card import-card">
      <h3>{t("Add a live station")}</h3>
      <p className="muted">{t("Check the detections of a Wildlife Network Base or a BirdNET-Go station on the same network: Studio reads them, with their clips, into this project for review, export and publishing. Nothing is sent to the station.")}
        {" "}{t("BirdNET-Pi has no stable read API, so it cannot be added live: copy its files and use Import results instead.")}</p>
      <div className="row tight">
        <input value={address} onChange={(e) => setAddress(e.target.value)} placeholder="192.168.1.20" list="live-recent" aria-label={t("Station address")} onKeyDown={(e) => e.key === "Enter" && add()} />
        <datalist id="live-recent">{recent.map((a) => <option key={a} value={a} />)}</datalist>
        <label className="small">{t("From")} <input type="date" value={from} onChange={(e) => setFrom(e.target.value)} /></label>
        <label className="small">{t("To")} <input type="date" value={to} onChange={(e) => setTo(e.target.value)} /></label>
        <button onClick={add} disabled={!!busy || !address.trim()}>{busy === "new" ? t("Reading the station...") : t("Import detections")}</button>
      </div>
      <p className="muted small">{t("The address is the one you open in a browser on this network, for example 192.168.1.20 or wnbase.local. Dates are the station's own calendar days.")}</p>
      {stations.length > 0 && <table className="grid">
        <thead><tr><th>{t("Station")}</th><th>{t("Address")}</th><th>{t("Newest day imported")}</th><th /></tr></thead>
        <tbody>{stations.map((s) => <tr key={s.key}>
          <td>{s.name}<small className="muted block">{s.kind === "base" ? "Wildlife Network Base" : "BirdNET-Go"}</small></td>
          <td className="small">{s.address.replace(/^http:\/\//, "")}</td>
          <td className="small">{s.last ?? ""}</td>
          <td><button onClick={() => run(s.key, () => api.liveRefresh(s.key))} disabled={!!busy} title={t("Adds the detections made since the newest one imported. Nothing is added twice.")}>{busy === s.key ? t("Reading the station...") : t("Refresh")}</button></td>
        </tr>)}</tbody>
      </table>}
    </div>
  );
}
