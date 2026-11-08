import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { useT } from "./i18n";

// Checks the latest GitHub release once on startup (not in dev). Quiet: no network or no update shows nothing.
export default function UpdateBanner() {
  const t = useT();
  const [update, setUpdate] = useState<Update | null>(null);
  const [state, setState] = useState<"idle" | "downloading" | "installing" | "error">("idle");
  const [pct, setPct] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (import.meta.env.DEV) return;
    let gone = false;
    check().then((u) => { if (!gone && u) setUpdate(u); }).catch(() => { /* offline or no release: stay quiet */ });
    return () => { gone = true; };
  }, []);

  if (!update) return null;

  async function install() {
    if (!update) return;
    setState("downloading"); setError(null);
    let total = 0, got = 0;
    try {
      await update.downloadAndInstall((e) => {
        if (e.event === "Started") { total = e.data.contentLength ?? 0; got = 0; setPct(total ? 0 : null); }
        else if (e.event === "Progress") { got += e.data.chunkLength; if (total) setPct(Math.min(100, Math.round((got / total) * 100))); }
        else if (e.event === "Finished") setState("installing");
      });
      await relaunch();
    } catch (e) {
      setState("error"); setError(String(e));
    }
  }

  const busy = state === "downloading" || state === "installing";
  return (
    <div className="update-banner" role="status">
      <div>
        <b>{t("Version {version} is available", { version: update.version })}</b>
        {state === "downloading" && <small>{pct !== null ? t("Downloading {pct}%", { pct }) : t("Downloading")}</small>}
        {state === "installing" && <small>{t("Installing, Studio will restart")}</small>}
        {state === "error" && <small className="error">{t("Update failed: {error}", { error: error ?? "" })}</small>}
      </div>
      {state === "downloading" && pct !== null && <div className="bar"><div style={{ width: `${pct}%` }} /></div>}
      <div className="row">
        <button className="primary" disabled={busy} onClick={install}>{state === "error" ? t("Try again") : t("Install and restart")}</button>
        <button disabled={busy} onClick={() => setUpdate(null)}>{t("Later")}</button>
      </div>
    </div>
  );
}
