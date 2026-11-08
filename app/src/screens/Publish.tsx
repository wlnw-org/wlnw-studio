import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, type DatasetInfo, type PublishReport, type Summary } from "../api";

const LICENSES: [string, string][] = [["CC0-1.0", "CC0 (public domain)"], ["CC-BY-4.0", "CC BY 4.0 (credit required)"], ["CC-BY-NC-4.0", "CC BY-NC 4.0 (non-commercial)"]];

/** Describe the dataset once; export it for GBIF and other tools, or publish it straight to Wildlife Network. */
export default function Publish({ summary, refresh, onError }: { summary: Summary; refresh: () => void; onError: (e: string) => void }) {
  const [info, setInfo] = useState<DatasetInfo>(summary.dataset);
  const [saved, setSaved] = useState(true);
  const [msg, setMsg] = useState<string | null>(null);
  const [dataset, setDataset] = useState("");
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [sent, setSent] = useState<[number, number] | null>(null);
  const [report, setReport] = useState<PublishReport | null>(null);

  useEffect(() => { const un = listen<[number, number]>("publish-progress", (e) => setSent(e.payload)); return () => { un.then((f) => f()); }; }, []);

  const set = (patch: Partial<DatasetInfo>) => { setInfo({ ...info, ...patch }); setSaved(false); };
  async function saveInfo() { try { await api.setSetting("dataset", JSON.stringify(info)); setSaved(true); refresh(); } catch (e) { onError(String(e)); } }

  async function exportAs(format: string) {
    if (!saved) await saveInfo();
    const base = info.title.replace(/[^\w-]+/g, "-").slice(0, 60) || "detections";
    let out: string | null = null;
    if (format === "raven") { const d = await open({ title: "Folder for Raven and Audacity tables", directory: true }); out = typeof d === "string" ? d : null; }
    else {
      const ext = { dwca: "zip", wdx: "ndjson", csv: "csv", kaleidoscope: "csv" }[format]!;
      out = await save({ title: "Export", defaultPath: format === "kaleidoscope" ? "id.csv" : `${base}${format === "dwca" ? "-dwca" : ""}.${ext}`, filters: [{ name: format === "kaleidoscope" ? "Kaleidoscope id.csv" : format.toUpperCase(), extensions: [ext] }] });
    }
    if (!out) return;
    setBusy(true); setMsg(null);
    try { const n = await api.exportTo(format, out); setMsg(`${n.toLocaleString()} detections exported to ${out}.`); } catch (e) { onError(String(e)); }
    setBusy(false);
  }

  async function publish(submit: boolean) {
    if (!saved) await saveInfo();
    setBusy(true); setReport(null); setSent(null);
    try { setReport(await api.publishTo(dataset, key, submit)); } catch (e) { onError(String(e)); }
    setBusy(false);
  }

  return (
    <section>
      <header className="head"><div><h2>Export and publish</h2>
        <p className="muted">Only accepted detections leave Studio: in range for their place and week, at or above each species&apos; reviewed threshold, and not marked wrong. Detections you marked yes are always included.</p></div></header>

      <div className="card form">
        <h3>About this dataset</h3>
        <label className="field">Title<input value={info.title} onChange={(e) => set({ title: e.target.value })} /></label>
        <label className="field">Institution<input value={info.institution ?? ""} onChange={(e) => set({ institution: e.target.value || null })} /></label>
        <label className="field">People to credit, one per line<textarea rows={3} value={info.people.join("\n")} onChange={(e) => set({ people: e.target.value.split("\n").map((s) => s.trim()).filter(Boolean) })} /></label>
        <label className="field">How it was collected<textarea rows={3} value={info.description ?? ""} onChange={(e) => set({ description: e.target.value || null })} /></label>
        <label className="field">Licence
          <select value={info.license} onChange={(e) => set({ license: e.target.value })}>{LICENSES.map(([k, l]) => <option key={k} value={k}>{l}</option>)}</select>
        </label>
        {!saved && <button onClick={saveInfo}>Save details</button>}
      </div>

      <div className="card">
        <h3>Export files</h3>
        <div className="exports">
          <button disabled={busy} onClick={() => exportAs("dwca")}><b>Darwin Core Archive</b><small>For GBIF: upload to your IPT or send to your GBIF node</small></button>
          <button disabled={busy} onClick={() => exportAs("wdx")}><b>WDX</b><small>Open detection format, one event per line</small></button>
          <button disabled={busy} onClick={() => exportAs("raven")}><b>Raven and Audacity</b><small>Selection tables and labels next to each recording</small></button>
          <button disabled={busy} onClick={() => exportAs("csv")}><b>CSV</b><small>One row per detection, for R, Python or a spreadsheet</small></button>
          <button disabled={busy} onClick={() => exportAs("kaleidoscope")}><b>Kaleidoscope id.csv</b><small>For Kaleidoscope users: your yes answers go in MANUAL ID</small></button>
        </div>
        {msg && <p className="note">{msg}</p>}
      </div>

      <div className="card form">
        <h3>Publish to Wildlife Network</h3>
        <p className="muted">Create a dataset at <button className="link inline" onClick={() => openUrl("https://wlnw.org/datasets/new")}>wlnw.org/datasets</button> (sign in with ORCID), then create its upload key on the dataset page and paste both here. The site checks every record again and credits the dataset by name wherever it is used. Sending again is safe: duplicates are ignored.</p>
        <label className="field">Dataset id<input value={dataset} onChange={(e) => setDataset(e.target.value)} placeholder="park-acoustic-2024-a1b2c3" /></label>
        <label className="field">Upload key<input type="password" value={key} onChange={(e) => setKey(e.target.value)} placeholder="wnds_..." autoComplete="off" /></label>
        <div className="row">
          <button disabled={busy || !dataset || !key} onClick={() => publish(false)}>Send records</button>
          <button className="primary" disabled={busy || !dataset || !key} onClick={() => publish(true)}>Send and submit</button>
        </div>
        {busy && sent && <p className="muted">Sent {sent[0].toLocaleString()} of {sent[1].toLocaleString()}...</p>}
        {report && <p className="note">Sent {report.sent.toLocaleString()}: {report.added.toLocaleString()} added, {report.duplicates.toLocaleString()} already there{report.left_out ? `, ${report.left_out.toLocaleString()} left out by the site (name not recognised or below the dataset's cut-off)` : ""}.
          {report.status === "live" && " The dataset is live."}{report.status === "review" && " Submitted: a reviewer will look at it."}</p>}
      </div>
    </section>
  );
}
