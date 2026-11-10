import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, type DatasetInfo, type PublishReport, type Summary } from "../api";
import { useT } from "../i18n";

const LICENSES: [string, string][] = [["CC0-1.0", "CC0 (public domain)"], ["CC-BY-4.0", "CC BY 4.0 (credit required)"], ["CC-BY-NC-4.0", "CC BY-NC 4.0 (non-commercial)"]];

/** Describe the dataset once; export it for GBIF and other tools, or publish it straight to Wildlife Network. */
export default function Publish({ summary, refresh, onError }: { summary: Summary; refresh: () => void; onError: (e: string) => void }) {
  const t = useT();
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
    if (format === "raven") { const d = await open({ title: t("Folder for Raven and Audacity tables"), directory: true }); out = typeof d === "string" ? d : null; }
    else if (format === "camtrapdp") { const d = await open({ title: t("Empty folder for the Camtrap DP package"), directory: true }); out = typeof d === "string" ? d : null; }
    else {
      const occ = format.startsWith("occupancy");
      const ext = occ ? "csv" : { dwca: "zip", wdx: "ndjson", csv: "csv", kaleidoscope: "csv" }[format]!;
      const name = format === "kaleidoscope" ? "id.csv" : occ ? `${base}-occupancy-${format === "occupancy-day" ? "days" : "weeks"}.csv` : `${base}${format === "dwca" ? "-dwca" : ""}.${ext}`;
      out = await save({ title: t("Export"), defaultPath: name, filters: [{ name: format === "kaleidoscope" ? "Kaleidoscope id.csv" : occ ? "CSV" : format.toUpperCase(), extensions: [ext] }] });
    }
    if (!out) return;
    setBusy(true); setMsg(null);
    try { const n = await api.exportTo(format, out); setMsg(format.startsWith("occupancy") ? t("Occupancy table saved to {path}, built from {n} detections, with minutes recorded per site in {effort}.", { n: n.toLocaleString(), path: out, effort: out.replace(/\.csv$/i, "") + "-effort.csv" }) : format === "camtrapdp" ? t("{n} observations exported to {path}. Images with people were left out, and so were images without a time or a camera without a place.", { n: n.toLocaleString(), path: out }) : t("{n} detections exported to {path}.", { n: n.toLocaleString(), path: out })); } catch (e) { onError(String(e)); }
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
      <header className="head"><div><h2>{t("Export and publish")}</h2>
        <p className="muted">{t("Only accepted detections leave Studio: in range for their place and week, at or above each species' reviewed threshold, and not marked wrong. Detections you marked yes are always included.")}</p></div></header>

      <div className="card form">
        <h3>{t("About this dataset")}</h3>
        <label className="field">{t("Title")}<input value={info.title} onChange={(e) => set({ title: e.target.value })} /></label>
        <label className="field">{t("Institution")}<input value={info.institution ?? ""} onChange={(e) => set({ institution: e.target.value || null })} /></label>
        <label className="field">{t("People to credit, one per line")}<textarea rows={3} value={info.people.join("\n")} onChange={(e) => set({ people: e.target.value.split("\n").map((s) => s.trim()).filter(Boolean) })} /></label>
        <label className="field">{t("How it was collected")}<textarea rows={3} value={info.description ?? ""} onChange={(e) => set({ description: e.target.value || null })} /></label>
        <label className="field">{t("Licence")}
          <select value={info.license} onChange={(e) => set({ license: e.target.value })}>{LICENSES.map(([k, l]) => <option key={k} value={k}>{t(l)}</option>)}</select>
        </label>
        {!saved && <button onClick={saveInfo}>{t("Save details")}</button>}
      </div>

      <div className="card">
        <h3>{t("Export files")}</h3>
        <div className="exports">
          <button disabled={busy} onClick={() => exportAs("dwca")}><b>Darwin Core Archive</b><small>{t("For GBIF: upload to your IPT or send to your GBIF node")}</small></button>
          <button disabled={busy} onClick={() => exportAs("wdx")}><b>WDX</b><small>{t("Open detection format, one event per line")}</small></button>
          <button disabled={busy} onClick={() => exportAs("raven")}><b>{t("Raven and Audacity")}</b><small>{t("Selection tables and labels next to each recording")}</small></button>
          <button disabled={busy} onClick={() => exportAs("csv")}><b>CSV</b><small>{t("One row per detection or camera-trap box, for R, Python or a spreadsheet")}</small></button>
          <button disabled={busy} onClick={() => exportAs("occupancy-day")}><b>{t("Occupancy by day")}</b><small>{t("Species by site by day: 1 detected, 0 recorded but not detected, NA not recorded. For occupancy models in R (unmarked, spOccupancy), with a minutes-recorded table")}</small></button>
          <button disabled={busy} onClick={() => exportAs("occupancy-week")}><b>{t("Occupancy by week")}</b><small>{t("The same table with Monday-to-Sunday weeks, for long deployments")}</small></button>
          <button disabled={busy} onClick={() => exportAs("kaleidoscope")}><b>Kaleidoscope id.csv</b><small>{t("For Kaleidoscope users: your yes answers go in MANUAL ID")}</small></button>
          {summary.images && <button disabled={busy} onClick={() => exportAs("camtrapdp")}><b>Camtrap DP</b><small>{t("Camera-trap images: deployments, media and observations, for GBIF, Agouti or camtraptor")}</small></button>}
        </div>
        {msg && <p className="note">{msg}</p>}
      </div>

      <div className="card form">
        <h3>{t("Publish to Wildlife Network")}</h3>
        <p className="muted">{t("Create a dataset at")} <button className="link inline" onClick={() => openUrl("https://wlnw.org/datasets/new")}>wlnw.org/datasets</button> {t("(sign in with ORCID), then create its upload key on the dataset page and paste both here. The site checks every record again and credits the dataset by name wherever it is used. Sending again is safe: duplicates are ignored.")}</p>
        <label className="field">{t("Dataset id")}<input value={dataset} onChange={(e) => setDataset(e.target.value)} placeholder="park-acoustic-2024-a1b2c3" /></label>
        <label className="field">{t("Upload key")}<input type="password" value={key} onChange={(e) => setKey(e.target.value)} placeholder="wnds_..." autoComplete="off" /></label>
        <div className="row">
          <button disabled={busy || !dataset || !key} onClick={() => publish(false)}>{t("Send records")}</button>
          <button className="primary" disabled={busy || !dataset || !key} onClick={() => publish(true)}>{t("Send and submit")}</button>
        </div>
        {busy && sent && <p className="muted">{t("Sent {done} of {total}...", { done: sent[0].toLocaleString(), total: sent[1].toLocaleString() })}</p>}
        {report && <p className="note">{report.left_out
          ? t("Sent {sent}: {added} added, {dupes} already there, {left} left out by the site (name not recognised or below the dataset's cut-off).", { sent: report.sent.toLocaleString(), added: report.added.toLocaleString(), dupes: report.duplicates.toLocaleString(), left: report.left_out.toLocaleString() })
          : t("Sent {sent}: {added} added, {dupes} already there.", { sent: report.sent.toLocaleString(), added: report.added.toLocaleString(), dupes: report.duplicates.toLocaleString() })}
          {report.status === "live" && " " + t("The dataset is live.")}{report.status === "review" && " " + t("Submitted: a reviewer will look at it.")}</p>}
      </div>
    </section>
  );
}
