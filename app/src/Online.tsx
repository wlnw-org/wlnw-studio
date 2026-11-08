import { useCallback, useEffect, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { LANGS, setLang, useLang, useT, type Lang } from "./i18n";
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
  const t = useT();
  const lang = useLang();
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
      <summary>{t("Settings")}</summary>
      <label className="switch">
        <span>{t("Language")}</span>
        <select value={lang} onChange={(e) => setLang(e.target.value as Lang)} aria-label={t("Language")}>
          {LANGS.map(([k, name]) => <option key={k} value={k}>{name}</option>)}
        </select>
      </label>
      <label className="switch">
        <input type="checkbox" checked={on} onChange={(e) => toggle(e.target.checked)} />
        <span>{t("Use Wildlife Network when online")}</span>
      </label>
      <small className="muted block">{t("Shows which species are known near each recorder, usual arrival weeks, protected areas and species photos from wlnw.org. Studio sends only each recorder's position rounded to about 10 km and species names. Never audio, files or results. Answers are kept in the project for 30 days, so they also work offline.")}</small>
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
  const t = useT();
  if (s === "unusual") return <span className="wn-mark" title={t("Not in GBIF records within 25 km of the recorders it was detected at: check carefully")}>!</span>;
  if (s === "few") return <span className="wn-mark soft" title={t("Not in GBIF records within 25 km, but its group has few records nearby")}>?</span>;
  return null;
}

const GROUP: Record<string, string> = { avian: "birds", bat: "bats", amphibian: "amphibians", insect: "insects" };
const day = (iso: string) => { const d = new Date(iso + "T00:00:00Z"); return isNaN(d.getTime()) ? iso : d.toLocaleDateString(undefined, { day: "numeric", month: "short", timeZone: "UTC" }); };

/** The Wildlife Network box in Review: one species at the recorder of the clip being reviewed. */
export function SpeciesBox({ d }: { d: Detection }) {
  const t = useT();
  const [info, setInfo] = useState<SpeciesInfo | null>(null);
  const { scientific, model, deployment_id } = d;
  const load = useCallback(() => {
    api.onlineSpecies(scientific, model, deployment_id).then(setInfo).catch(() => setInfo(null));
  }, [scientific, model, deployment_id]);
  useEffect(() => { setInfo(null); }, [scientific, deployment_id]);
  useOnlineRefresh(load);
  if (!info) return null;
  const noPlace = d.lat == null || d.lon == null;
  const group = info.group && GROUP[info.group] ? t(GROUP[info.group]) : null;
  let line: ReactNode = null;
  if (info.status === "known") line = <>{t("Recorded within 25 km: yes, {n} GBIF records.", { n: (info.records ?? 0).toLocaleString() })}</>;
  else if (info.status === "unusual") line = <span className="wn-warn">{t("Not in GBIF records within 25 km of this recorder: check carefully.")}</span>;
  else if (info.status === "few") line = <span className="wn-soft">{group ? t("Few records of {group} nearby, and none of this species within 25 km.", { group }) : t("Few records of this group nearby, and none of this species within 25 km.")}</span>;
  else if (noPlace) line = <span className="muted">{t("Add this recorder's place to compare with records nearby.")}</span>;
  const a = info.arrival;
  const range = a && a.low && a.high && (a.low !== a.week || a.high !== a.week) ? { low: day(a.low), high: day(a.high) } : null;
  const arrival = a && <>{a.region
    ? range ? t("Usually arrives in {region} in the week of {week} (likely between {low} and {high}).", { region: a.region, week: day(a.week), ...range })
      : t("Usually arrives in {region} in the week of {week}.", { region: a.region, week: day(a.week) })
    : range ? t("Usually arrives in the week of {week} (likely between {low} and {high}).", { week: day(a.week), ...range })
      : t("Usually arrives in the week of {week}.", { week: day(a.week) })}</>;
  if (!line && !arrival && !info.line && !info.image) return null;
  return (
    <div className={`wn-box ${info.status}`}>
      {info.image && <img src={info.image} alt={info.title ?? ""} title={info.credit ?? ""} />}
      <div>
        <small className="wn-head">Wildlife Network</small>
        {line && <p>{line}</p>}
        {arrival && <p>{arrival}</p>}
        {info.line && <p className="muted small">{info.line}</p>}
        <button className="link inline small" onClick={() => open(info.url)}>{t("Open on wlnw.org")}</button>
      </div>
    </div>
  );
}

/** Recordings: per recorder with a position, species known nearby and the nearest protected areas. */
export function AroundRecorders({ deps }: { deps: Deployment[] }) {
  const t = useT();
  const [rows, setRows] = useState<Around[]>([]);
  const load = useCallback(() => { api.onlineAround().then(setRows).catch(() => setRows([])); }, []);
  useOnlineRefresh(load);
  const shown = rows.filter((r) => r.species != null || r.areas.length > 0);
  if (shown.length === 0) return null;
  const name = (id: number) => deps.find((d) => d.id === id)?.name ?? t("Recorder");
  return (
    <div className="card around">
      <h3>{shown.length === 1 ? t("Around this recorder") : t("Around these recorders")}</h3>
      {shown.map((r) => (
        <p key={r.deployment_id}>
          <b>{name(r.deployment_id)}</b>
          {r.species != null && <>: {t("{n} species known within 25 km in GBIF records.", { n: r.species.toLocaleString() + (r.more ? "+" : "") })}</>}
          {r.areas.length > 0 && <> {t("Nearest protected areas: {areas}.", { areas: r.areas.map((x) => `${x.name} (${Math.round(x.km)} km)`).join(", ") })}</>}
          {" "}<button className="link inline" onClick={() => open("https://wlnw.org/report/area")}>{t("Area report on wlnw.org")}</button>
        </p>
      ))}
      <small className="muted">{t("From wlnw.org, kept in this project. Turn this off in Settings.")}</small>
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
  const t = useT();
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
    if (!confirm(t("Delete every answer this computer has shared with Wildlife Network? This cannot be undone, and sharing is turned off."))) return;
    try { const n = await api.shareDelete(); setNote(t("Deleted {n} shared answers from wlnw.org. Sharing is off.", { n: n.toLocaleString() })); load(); }
    catch { setNote(t("Could not reach wlnw.org. Try again when you are online.")); }
  }
  if (!st) return null;
  return (
    <div className="share-setting">
      <label className="switch">
        <input type="checkbox" checked={st.enabled} onChange={(e) => toggle(e.target.checked)} />
        <span>{t("Share my checked answers to improve species ID (off unless you turn it on)")}</span>
      </label>
      <small className="muted block">
        {t("Helps build free regional classifiers for places that BirdNET and Perch know poorly. For each clip you answered yes or no, or a sound you named, Studio sends the species, your answer, the clip's Perch embedding (1,536 numbers that describe the sound; not audio and cannot be played back), the recorder's place rounded to about 100 km, and the year and week. Never audio, file or project names, exact times, or anything about you. Shared answers are open data (CC0).")}{" "}
        <button className="link inline" onClick={() => openUrl("https://wlnw.org/privacy#shared-answers").catch(() => {})}>{t("Details")}</button>
      </small>
      {(st.enabled || st.shared > 0) && <small className="muted block">{st.enabled && st.pending > 0
        ? t("This project: {shared} shared, {pending} to send.", { shared: st.shared.toLocaleString(), pending: st.pending.toLocaleString() })
        : t("This project: {shared} shared.", { shared: st.shared.toLocaleString() })}</small>}
      {st.has_key && <button className="link small" onClick={remove}>{t("Delete everything I shared")}</button>}
      {note && <small className="block">{note}</small>}
    </div>
  );
}
