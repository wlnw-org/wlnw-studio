import { useEffect, useState } from "react";
import { api, fmtDuration, fmtWhen, type HistoryEntry, type Summary } from "../api";
import { useT } from "../i18n";

const KIND: Record<string, string> = { run: "Processing", scan: "Recordings added", train: "Classifier trained", export: "Export", publish: "Published", import: "Results imported", agreement: "Second reviewer" };
const STATUS: Record<string, string> = { running: "Running", finished: "Finished", paused: "Paused", interrupted: "Interrupted", failed: "Failed" };

function took(e: HistoryEntry) {
  if (!e.finished_at) return null;
  const s = (new Date(e.finished_at.replace(" ", "T") + "Z").getTime() - new Date(e.started_at.replace(" ", "T") + "Z").getTime()) / 1000;
  return s >= 1 ? fmtDuration(s) : null;
}

/** Everything done in this project, newest first. Stored in the project file, so it travels with it. */
export default function History({ summary, onError }: { summary: Summary; onError: (e: string) => void }) {
  const t = useT();
  const [items, setItems] = useState<HistoryEntry[] | null>(null);
  useEffect(() => {
    let live = true;
    const load = () => api.history().then((h) => { if (live) setItems(h); }).catch((e) => onError(String(e)));
    load();
    const t = summary.running ? setInterval(load, 5000) : undefined;
    return () => { live = false; if (t) clearInterval(t); };
  }, [summary.running, onError]);

  return (
    <section>
      <header className="head">
        <div>
          <h2>{t("History")}</h2>
          <p className="muted">{t("What has been done in this project and when: processing runs, recordings added, training, exports and publishing. It is saved in the project file, so it moves with the project.")}</p>
        </div>
      </header>
      {items && items.length === 0 && <div className="empty">{t("Nothing yet. Each processing run, export and publish will be listed here.")}</div>}
      {items && items.length > 0 && (
        <ol className="history">
          {items.map((e) => {
            const d = took(e);
            const speed = typeof e.detail?.speed_x === "number" && e.detail.speed_x > 0 ? Math.round(e.detail.speed_x) : null;
            return (
              <li key={e.id} className={`h h-${e.status}`}>
                <div className="h-top">
                  <b>{KIND[e.kind] ? t(KIND[e.kind]) : e.kind}</b>
                  <span className={`status st-${e.status}`}>{e.status === "running" && summary.running ? t("Running now") : e.status === "running" ? t(STATUS.interrupted) : STATUS[e.status] ? t(STATUS[e.status]) : e.status}</span>
                  <span className="muted small">{fmtWhen(e.started_at)}{d ? t(", took {time}", { time: d }) : ""}{speed ? t(", {x}x real time", { x: speed }) : ""}</span>
                </div>
                <p>{e.summary}</p>
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}
