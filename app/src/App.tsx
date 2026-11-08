import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, type Progress, type Summary } from "./api";
import Recordings from "./screens/Recordings";
import Process from "./screens/Process";
import Review from "./screens/Review";
import Publish from "./screens/Publish";
import Files from "./screens/Files";
import History from "./screens/History";
import { OnlineSetting } from "./Online";
import Images from "./screens/Images";
import { ErrorHelp, FeedbackLink, HelpLink, ScreenHelp } from "./Feedback";

type Step = "recordings" | "process" | "review" | "images" | "publish" | "files" | "history";
const STEPS: { id: Step; label: string; hint: string }[] = [
  { id: "recordings", label: "Recordings", hint: "Add a drive or folder" },
  { id: "process", label: "Find species", hint: "On this computer, nothing uploaded" },
  { id: "review", label: "Review", hint: "Check clips, teach the classifier" },
  { id: "publish", label: "Export and publish", hint: "GBIF, Wildlife Network, files" },
];
const RECENT = "wlnw.recent";

function recent(): string[] { try { return JSON.parse(localStorage.getItem(RECENT) ?? "[]"); } catch { return []; } }
function remember(p: string) { try { localStorage.setItem(RECENT, JSON.stringify([p, ...recent().filter((x) => x !== p)].slice(0, 6))); } catch { /* storage unavailable */ } }

export default function App() {
  const [summary, setSummary] = useState<Summary | null>(null);
  const [step, setStep] = useState<Step>("recordings");
  const [error, setError] = useState<string | null>(null);
  const [live, setLive] = useState<Progress | null>(null);
  const [filesStatus, setFilesStatus] = useState<string | undefined>(undefined);

  const refresh = useCallback(async () => {
    try { setSummary(await api.summary()); } catch (e) { setError(String(e)); }
  }, []);

  // A project opened at launch.
  useEffect(() => { api.summary().then(setSummary).catch(() => {}); }, []);

  useEffect(() => {
    const un = [
      listen<Progress>("run-progress", (e) => setLive(e.payload)),
      listen<string | null>("run-done", (e) => { if (e.payload) setError(e.payload); refresh(); }),
    ];
    return () => { un.forEach((p) => p.then((f) => f())); };
  }, [refresh]);

  async function openFile(file: string) {
    setError(null);
    try { await api.openProject(file); remember(file); await refresh(); setStep("recordings"); } catch (e) { setError(String(e)); }
  }
  async function newProject() {
    const file = await save({ title: "New project", defaultPath: "Field season.wlnw", filters: [{ name: "Studio project", extensions: ["wlnw"] }] });
    if (file) openFile(file);
  }
  async function openProject() {
    const file = await open({ title: "Open project", multiple: false, filters: [{ name: "Studio project", extensions: ["wlnw"] }] });
    if (typeof file === "string") openFile(file);
  }

  if (!summary) {
    return (
      <main className="home">
        <div className="home-card">
          <img src="/logo.svg" alt="" className="home-logo" />
          <h1>Wildlife Network Studio</h1>
          <p className="lead">Point it at your recordings. It finds the species on this computer, helps you check them in minutes, and exports them for GBIF or publishes them to Wildlife Network.</p>
          <div className="row">
            <button className="primary" onClick={newProject}>New project</button>
            <button onClick={openProject}>Open project</button>
          </div>
          {recent().length > 0 && (
            <div className="recent">
              <h3>Recent</h3>
              {recent().map((p) => <button key={p} className="link" onClick={() => openFile(p)} title={p}>{p.split(/[\\/]/).pop()}</button>)}
            </div>
          )}
          {error && <p className="error">{error} <ErrorHelp /></p>}
          <OnlineSetting />
          <div className="row help-links"><HelpLink /><FeedbackLink /></div>
        </div>
      </main>
    );
  }

  const prog = live ?? summary.progress;
  return (
    <div className="shell">
      <aside className="rail">
        <div className="brand"><img src="/logo.svg" alt="" /><span>Studio</span></div>
        <div className="project" title={summary.path}><small>Project</small><b>{summary.name}</b></div>
        <nav>
          {(summary.images ? [...STEPS.slice(0, 3), { id: "images" as Step, label: "Images", hint: "Camera-trap photos by species" }, STEPS[3]] : STEPS).map((s, i) => (
            <button key={s.id} className={step === s.id ? "step active" : "step"} onClick={() => setStep(s.id)}>
              <span className="num">{i + 1}</span>
              <span><b>{s.label}</b><small>{s.hint}</small></span>
            </button>
          ))}
        </nav>
        <nav className="secondary">
          <button className={step === "files" ? "step active" : "step"} onClick={() => { setFilesStatus(undefined); setStep("files"); }}>
            <span><b>Files</b><small>Every recording and its status</small></span>
          </button>
          <button className={step === "history" ? "step active" : "step"} onClick={() => setStep("history")}>
            <span><b>History</b><small>Runs, exports and publishing</small></span>
          </button>
        </nav>
        {summary.running && <div className="running"><span className="pulse" /> {prog.files_total > 0 && prog.files_done < prog.files_total ? `Processing ${prog.files_done} of ${prog.files_total} recordings` : `Processing ${prog.images_done} of ${prog.images_total} images`}</div>}
        <OnlineSetting />
        <HelpLink />
        <FeedbackLink />
        <button className="link small" onClick={() => { setSummary(null); setLive(null); }}>Close project</button>
      </aside>
      <main className="content">
        <ScreenHelp step={step} />
        {error && <div className="banner error" onClick={() => setError(null)}>{error} <ErrorHelp /> <span className="dismiss">Dismiss</span></div>}
        {step === "recordings" && <Recordings summary={summary} refresh={refresh} onError={setError} next={() => setStep("process")} />}
        {step === "process" && <Process summary={summary} live={live} refresh={refresh} onError={setError} next={() => setStep("review")} showFiles={(st) => { setFilesStatus(st); setStep("files"); }} />}
        {step === "review" && <Review summary={summary} refresh={refresh} onError={setError} />}
        {step === "images" && <Images summary={summary} onError={setError} />}
        {step === "publish" && <Publish summary={summary} refresh={refresh} onError={setError} />}
        {step === "files" && <Files key={filesStatus ?? ""} summary={summary} refresh={refresh} onError={setError} initialStatus={filesStatus} />}
        {step === "history" && <History summary={summary} onError={setError} />}
      </main>
    </div>
  );
}
