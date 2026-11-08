import { useCallback, useEffect, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, type Around, type Deployment, type Detection, type OnlineStatus, type SpeciesInfo } from "./api";

/** Fired in the page when the setting changes, so open screens refresh. */
const CHANGED = "wlnw-online-setting";
const open = (url: string) => { openUrl(url).catch(() => {}); };

/** Re-runs `load` when the setting changes or new answers from wlnw.org arrive. */
function useOnlineRefresh(load: () => void) {
  useEffect(() => {
    load();
    const un = listen("online-updated", () => load());
    window.addEventListener(CHANGED, load);
    return () => { un.then((f) => f()); window.removeEventListener(CHANGED, load); };
  }, [load]);
}

/** Small Settings area: "Use Wildlife Network when online" and exactly what it sends. */
export function OnlineSetting() {
  const [on, setOn] = useState<boolean | null>(null);
  useEffect(() => { api.onlineEnabled().then(setOn).catch(() => {}); }, []);
  async function toggle(v: boolean) {
    setOn(v);
    try { await api.setOnlineEnabled(v); } catch { /* the switch still applies for this session */ }
    window.dispatchEvent(new Event(CHANGED));
    if (v) api.onlinePrefetch().catch(() => {});
  }
  if (on === null) return null;
  return (
    <details className="settings">
      <summary>Settings</summary>
      <label className="switch">
        <input type="checkbox" checked={on} onChange={(e) => toggle(e.target.checked)} />
        <span>Use Wildlife Network when online</span>
      </label>
      <small className="muted block">Shows which species are known near each recorder, usual arrival weeks, protected areas and species photos from wlnw.org.
        Studio sends only each recorder&apos;s position rounded to about 10 km and species names. Never audio, files or results.
        Answers are kept in the project for 30 days, so they also work offline.</small>
      <ShareSetting />
    </details>
  );
}

/** Background fill of the project's cache (does nothing when the setting is off). Call once when a screen opens. */
export function usePrefetch() {
  useEffect(() => { api.onlinePrefetch().catch(() => {}); }, []);
}

/** Species-list markers: label id to "unusual" or "few". Empty when off or nothing is known yet. */
export function useOnlineFlags(): Record<string, OnlineStatus> {
  const [flags, setFlags] = useState<Record<string, OnlineStatus>>({});
  const load = useCallback(() => { api.onlineFlags().then(setFlags).catch(() => setFlags({})); }, []);
  useOnlineRefresh(load);
  return flags;
}

export function FlagMark({ s }: { s: OnlineStatus | undefined }) {
  if (s === "unusual") return <span className="wn-mark" title="Not in GBIF records within 25 km of the recorders it was detected at: check carefully">!</span>;
  if (s === "few") return <span className="wn-mark soft" title="Not in GBIF records within 25 km, but its group has few records nearby">?</span>;
  return null;
}

const GROUP: Record<string, string> = { avian: "birds", bat: "bats", amphibian: "amphibians", insect: "insects" };
const day = (iso: string) => { const d = new Date(iso + "T00:00:00Z"); return isNaN(d.getTime()) ? iso : d.toLocaleDateString(undefined, { day: "numeric", month: "short", timeZone: "UTC" }); };

/** The Wildlife Network box in Review: one species at the recorder of the clip being reviewed. */
export function SpeciesBox({ d }: { d: Detection }) {
  const [info, setInfo] = useState<SpeciesInfo | null>(null);
  const { scientific, model, deployment_id } = d;
  const load = useCallback(() => {
    api.onlineSpecies(scientific, model, deployment_id).then(setInfo).catch(() => setInfo(null));
  }, [scientific, model, deployment_id]);
  useEffect(() => { setInfo(null); }, [scientific, deployment_id]);
  useOnlineRefresh(load);
  if (!info) return null;
  const noPlace = d.lat == null || d.lon == null;
  const group = info.group ? GROUP[info.group] : null;
  let line: ReactNode = null;
  if (info.status === "known") line = <>Recorded within 25 km: yes, {(info.records ?? 0).toLocaleString()} GBIF records.</>;
  else if (info.status === "unusual") line = <span className="wn-warn">Not in GBIF records within 25 km of this recorder: check carefully.</span>;
  else if (info.status === "few") line = <span className="wn-soft">Few records of {group ?? "this group"} nearby, and none of this species within 25 km.</span>;
  else if (noPlace) line = <span className="muted">Add this recorder&apos;s place to compare with records nearby.</span>;
  const a = info.arrival;
  const arrival = a && <>Usually arrives{a.region ? ` in ${a.region}` : ""} in the week of {day(a.week)}{a.low && a.high && (a.low !== a.week || a.high !== a.week) ? ` (likely between ${day(a.low)} and ${day(a.high)})` : ""}.</>;
  if (!line && !arrival && !info.line && !info.image) return null;
  return (
    <div className={`wn-box ${info.status}`}>
      {info.image && <img src={info.image} alt={info.title ?? ""} title={info.credit ?? ""} />}
      <div>
        <small className="wn-head">Wildlife Network</small>
        {line && <p>{line}</p>}
        {arrival && <p>{arrival}</p>}
        {info.line && <p className="muted small">{info.line}</p>}
        <button className="link inline small" onClick={() => open(info.url)}>Open on wlnw.org</button>
      </div>
    </div>
  );
}

/** Recordings: per recorder with a position, species known nearby and the nearest protected areas. */
export function AroundRecorders({ deps }: { deps: Deployment[] }) {
  const [rows, setRows] = useState<Around[]>([]);
  const load = useCallback(() => { api.onlineAround().then(setRows).catch(() => setRows([])); }, []);
  useOnlineRefresh(load);
  const shown = rows.filter((r) => r.species != null || r.areas.length > 0);
  if (shown.length === 0) return null;
  const name = (id: number) => deps.find((d) => d.id === id)?.name ?? "Recorder";
  return (
    <div className="card around">
      <h3>Around {shown.length === 1 ? "this recorder" : "these recorders"}</h3>
      {shown.map((r) => (
        <p key={r.deployment_id}>
          <b>{name(r.deployment_id)}</b>
          {r.species != null && <>: {r.species.toLocaleString()}{r.more ? "+" : ""} species known within 25 km in GBIF records.</>}
          {r.areas.length > 0 && <> Nearest protected areas: {r.areas.map((x) => `${x.name} (${Math.round(x.km)} km)`).join(", ")}.</>}
          {" "}<button className="link inline" onClick={() => open("https://wlnw.org/report/area")}>Area report on wlnw.org</button>
        </p>
      ))}
      <small className="muted">From wlnw.org, kept in this project. Turn this off in Settings.</small>
    </div>
  );
}

/** 64 hex characters from the system's secure random source: the only thing that can delete what this computer shared. */
function newKey(): string {
  const b = new Uint8Array(32);
  crypto.getRandomValues(b);
  return Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
}

/** Opt-in: "Share my checked answers" to train open regional classifiers, with exactly what is sent and a delete button. */
export function ShareSetting() {
  const [st, setSt] = useState<{ enabled: boolean; has_key: boolean; shared: number; pending: number } | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const load = useCallback(() => { api.shareStatus().then(setSt).catch(() => {}); }, []);
  useEffect(() => {
    load();
    const un = listen<string | null>("share-done", (e) => { if (e.payload) setNote(e.payload); load(); });
    return () => { un.then((f) => f()); };
  }, [load]);
  async function toggle(on: boolean) {
    try { await api.setShare(on, newKey()); load(); if (on) api.shareNow().catch(() => {}); } catch (e) { setNote(String(e)); }
  }
  async function remove() {
    if (!confirm("Delete every answer this computer has shared with Wildlife Network? This cannot be undone, and sharing is turned off.")) return;
    try { const n = await api.shareDelete(); setNote(`Deleted ${n.toLocaleString()} shared answers from wlnw.org. Sharing is off.`); load(); }
    catch { setNote("Could not reach wlnw.org. Try again when you are online."); }
  }
  if (!st) return null;
  return (
    <div className="share-setting">
      <label className="switch">
        <input type="checkbox" checked={st.enabled} onChange={(e) => toggle(e.target.checked)} />
        <span>Share my checked answers to improve species ID (off unless you turn it on)</span>
      </label>
      <small className="muted block">
        Helps build free regional classifiers for places that BirdNET and Perch know poorly. For each clip you answered yes or no, or a sound you named,
        Studio sends the species, your answer, the clip&apos;s Perch embedding (1,536 numbers that describe the sound; not audio and cannot be played back),
        the recorder&apos;s place rounded to about 100 km, and the year and week. Never audio, file or project names, exact times, or anything about you.
        Shared answers are open data (CC0). <button className="link inline" onClick={() => openUrl("https://wlnw.org/privacy#shared-answers").catch(() => {})}>Details</button>
      </small>
      {(st.enabled || st.shared > 0) && <small className="muted block">This project: {st.shared.toLocaleString()} shared{st.enabled && st.pending > 0 ? `, ${st.pending.toLocaleString()} to send` : ""}.</small>}
      {st.has_key && <button className="link small" onClick={remove}>Delete everything I shared</button>}
      {note && <small className="block">{note}</small>}
    </div>
  );
}
