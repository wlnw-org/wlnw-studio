import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, fmtHours, modelName, modelTag, type Deployment, type Summary } from "../api";
import MapPicker from "../MapPicker";
import { AroundRecorders, usePrefetch } from "../Online";

/** Add recordings, then confirm each recorder's place and time zone (filled in from the files when they carry it). */
export default function Recordings({ summary, refresh, onError, next }: { summary: Summary; refresh: () => void; onError: (e: string) => void; next: () => void }) {
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<string | null>(null);
  usePrefetch();
  async function add() {
    const folder = await open({ title: "Choose a folder or drive with recordings", directory: true, multiple: false });
    if (typeof folder !== "string") return;
    setBusy(true); setReport(null);
    try {
      const r = await api.scanFolder(folder);
      setReport(`${r.found.toLocaleString()} recordings found, ${r.added.toLocaleString()} new` +
        (r.images_found ? `, and ${r.images_found.toLocaleString()} camera-trap images, ${r.images_added.toLocaleString()} new` : "") +
        `, in ${r.deployments} ${r.deployments === 1 ? "recorder or camera" : "recorders and cameras"}.` +
        (r.without_time ? ` ${r.without_time.toLocaleString()} recordings have no date in their name or metadata and will be skipped for publishing.` : "") +
        (r.images_without_time ? ` ${r.images_without_time.toLocaleString()} images have no date in their EXIF data and will be skipped for publishing.` : ""));
      refresh();
    } catch (e) { onError(String(e)); }
    setBusy(false);
  }
  async function importResults(folder: boolean) {
    const f = await open(folder ? { title: "Choose a folder of results tables", directory: true, multiple: false }
      : { title: "Choose a results table", multiple: false, filters: [{ name: "Results table", extensions: ["csv", "txt"] }] });
    if (typeof f !== "string") return;
    setBusy(true); setReport(null);
    try {
      const r = await api.importResults(f);
      setReport(`${r.added.toLocaleString()} new detections from ${r.model.split(", ").map(modelTag).join(" and ")}: ${r.matched.toLocaleString()} of ${r.rows.toLocaleString()} rows matched to recordings in this project.` +
        (r.unmatched ? ` ${r.unmatched.toLocaleString()} rows did not match a recording here (for example ${r.unmatched_examples.slice(0, 3).join(", ")}). Add those recordings first, then import again: nothing is added twice.` : "") +
        (r.no_species ? ` ${r.no_species.toLocaleString()} rows had no species (noise or no ID)${r.unknown_names.length ? `, or a name Studio did not know: ${r.unknown_names.slice(0, 5).join(", ")}` : ""}.` : "") +
        (r.added ? " Review them on the Review screen." : ""));
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
          <h2>Recordings</h2>
          <p className="muted">Add a whole drive or SD card copy. Subfolders are searched; WAV, FLAC and MP3 recordings and JPEG and PNG camera-trap images are read. Files already added are skipped.</p>
        </div>
        <button className="primary" onClick={add} disabled={busy}>{busy ? "Reading files..." : "Add a folder or drive"}</button>
      </header>
      {report && <p className="note">{report}</p>}
      {deps.length === 0 ? (
        <div className="empty">No recordings yet. Studio reads the time and place written by AudioMoth and Song Meter recorders, and the date in file names such as SMM01234_20250601_053000.wav.</div>
      ) : (
        <>
          <p className="muted">{deps.length} {deps.length === 1 ? "recorder" : "recorders"}, {deps.reduce((s, d) => s + d.files, 0).toLocaleString()} files{cams ? `, ${deps.reduce((s, d) => s + d.images, 0).toLocaleString()} images` : ""}{hours > 0 ? `, ${fmtHours(hours)} processed` : ""}.
            {missing > 0 && <b className="warn"> {missing} need a place or time zone: the place decides which species are expected and which models run.</b>}</p>
          <table className="grid">
            <thead><tr><th>Recorder</th><th>Files</th><th>Place</th><th>Times are UTC +</th>{cams && <th title="Three-letter code (ISO 3166). SpeciesNet uses it to rule out species that do not live there.">Country</th>}<th>Models</th><th /></tr></thead>
            <tbody>{deps.map((d) => <Row key={d.id} d={d} cams={cams} onSaved={refresh} onError={onError} />)}</tbody>
          </table>
          {cams && <p className="muted small">Camera traps: set each camera&apos;s country (three letters, for example KEN, IND, BRA or USA, and the state code for the USA) so SpeciesNet can rule out species that do not live there. Changing it runs that camera&apos;s images again.</p>}
          <AroundRecorders deps={deps} />
          <div className="card import-card">
            <h3>Import results from another tool</h3>
            <p className="muted">Already ran BirdNET-Analyzer, BirdNET-Pi, BirdNET-Go or Kaleidoscope on these recordings? Import their results to review and publish them here without processing the audio again.
              Studio reads BirdNET-Analyzer CSV and Raven selection tables, BirdNET-Pi and BirdNET-Go detection exports, and Kaleidoscope id.csv. Each row is matched to a recording added above by its path or file name (BirdNET-Pi and Go rows by date and time).
              Kaleidoscope manual IDs count as review answers.</p>
            <div className="row tight">
              <button onClick={() => importResults(false)} disabled={busy}>Import a results file</button>
              <button onClick={() => importResults(true)} disabled={busy}>Import a folder of results</button>
            </div>
          </div>
          <div className="foot"><button className="primary" onClick={next}>Next: find species</button></div>
        </>
      )}
    </section>
  );
}

function Row({ d, cams, onSaved, onError }: { d: Deployment; cams: boolean; onSaved: () => void; onError: (e: string) => void }) {
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
      <td><input value={v.name} onChange={(e) => setV({ ...v, name: e.target.value })} aria-label="Recorder name" /><small className="muted block">{d.model ?? ""} {d.folder ? d.folder.split(/[\\/]/).slice(-2).join("/") : d.serial}</small></td>
      <td>{d.files}{d.images > 0 && <small className="muted block">{d.images.toLocaleString()} images</small>}</td>
      <td>
        <div className="place">
          <input className={d.lat == null ? "need" : ""} value={v.lat} onChange={(e) => setV({ ...v, lat: e.target.value })} placeholder="Latitude" inputMode="decimal" aria-label="Latitude" />
          <input className={d.lon == null ? "need" : ""} value={v.lon} onChange={(e) => setV({ ...v, lon: e.target.value })} placeholder="Longitude" inputMode="decimal" aria-label="Longitude" />
          <button className="small-btn" onClick={() => setMap(true)} title="Choose on a map">Map</button>
        </div>
      </td>
      <td><input className={!d.tz_known ? "need tz" : "tz"} value={v.tz} onChange={(e) => setV({ ...v, tz: e.target.value })} inputMode="decimal" aria-label="UTC offset in hours" title={d.tz_known ? "From the recording or confirmed" : "Please confirm: Song Meters usually record local time, AudioMoths UTC"} /></td>
      {cams && <td>{d.images > 0 && <>
        <input className={`country ${d.country ? "" : "need"}`} value={country} maxLength={3} onChange={(e) => setCountry(e.target.value)} placeholder="KEN" aria-label="Country code" />
        {country.trim().toUpperCase() === "USA" && <input className="country" value={state} maxLength={2} onChange={(e) => setState(e.target.value)} placeholder="State" aria-label="US state code" />}
        {countryDirty && <button className="small-btn" onClick={saveCountry}>Save</button>}
      </>}</td>}
      <td className="small">{[...d.models.map(modelName), ...(d.images > 0 ? ["SpeciesNet"] : [])].join(" + ") || "none"}{d.perch_region && <small className="muted block">Perch first in this region</small>}</td>
      <td>{dirty ? <button onClick={() => save()}>Save</button> : saved ? <span className="ok">Saved</span> : null}</td>
      {map && <MapPicker lat={d.lat} lon={d.lon} title={v.name} onClose={() => setMap(false)}
        onPick={(la, lo) => { const patch = { lat: String(la), lon: String(lo) }; setV({ ...v, ...patch }); setMap(false); save(patch); }} />}
    </tr>
  );
}
