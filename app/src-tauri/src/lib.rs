//! Wildlife Network Studio: the desktop shell around wlnw-engine. Every command opens its own connection to the project
//! file (SQLite in WAL mode), so the long processing run never blocks review or export.
use base64::Engine as _;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, State};
use wlnw_engine::models::{self, Classifier, ModelInfo};
use wlnw_engine::{agreement, camtrap, download, export, import, files, history, images, online, publish, review, run, scan, store::Project, wnc};

#[derive(Default)]
struct App {
    project: Mutex<Option<PathBuf>>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

type Models = (Vec<Arc<dyn Classifier>>, Vec<String>);
static MODELS: Mutex<Option<Arc<Models>>> = Mutex::new(None);
static DOWNLOADING: Mutex<Vec<String>> = Mutex::new(Vec::new());
static STOP_DOWNLOAD: AtomicBool = AtomicBool::new(false);

type R<T> = Result<T, String>;
fn err(e: impl std::fmt::Display) -> String { format!("{e:#}") }

/// Where downloaded models go: the app's data folder (per user, survives updates).
fn install_dir(app: &tauri::AppHandle) -> R<PathBuf> {
    use tauri::Manager;
    Ok(app.path().app_data_dir().map_err(err)?.join("models"))
}

/// Folders searched for model packs, first match wins: WLNW_MODELS, the downloads folder, the app bundle (and, in development builds, next to the app).
fn model_dirs(app: &tauri::AppHandle) -> Vec<PathBuf> {
    use tauri::Manager;
    let mut c: Vec<PathBuf> = std::env::var("WLNW_MODELS").ok().map(PathBuf::from).into_iter().collect();
    if let Ok(d) = install_dir(app) { c.push(d); }
    if let Ok(d) = app.path().resource_dir() { c.push(d.join("models")); }
    // Development builds also look next to the source tree; release builds never load models from folders anyone could write to.
    if cfg!(debug_assertions) { if let Ok(exe) = std::env::current_exe() { for a in exe.ancestors().skip(1).take(5) { c.push(a.join("models")); } } }
    c
}

fn models(app: &tauri::AppHandle) -> Arc<Models> {
    let mut g = MODELS.lock().unwrap();
    if g.is_none() {
        let (ms, mut errors) = models::available_in(&model_dirs(app));
        if ms.is_empty() { errors.push("No model is installed yet: download Perch below (free, about 410 MB).".into()); }
        *g = Some(Arc::new((ms, errors)));
    }
    g.clone().unwrap()
}
fn reload_models() { *MODELS.lock().unwrap() = None; }
fn have(app: &tauri::AppHandle) -> Vec<String> { models(app).0.iter().map(|m| m.info().id.clone()).collect() }

#[derive(Serialize)]
struct CatalogItem { #[serde(flatten)] pack: download::Pack, installed: bool, downloading: bool, size: u64 }

#[tauri::command]
fn model_catalog(app: tauri::AppHandle) -> Vec<CatalogItem> {
    let dirs = model_dirs(&app);
    let busy = DOWNLOADING.lock().unwrap().clone();
    download::CATALOG.iter().map(|p| CatalogItem {
        installed: dirs.iter().any(|d| download::installed(p, d)), downloading: busy.iter().any(|b| b == p.id),
        size: p.files.iter().map(|f| f.size).sum(), pack: p.clone(),
    }).collect()
}

#[derive(Clone, Serialize)]
struct DownloadEvent { id: String, done: u64, total: u64, finished: bool, error: Option<String> }

#[tauri::command]
fn download_model(app: tauri::AppHandle, id: String) -> R<()> {
    let pack = download::CATALOG.iter().find(|p| p.id == id).ok_or("unknown model")?;
    { let mut b = DOWNLOADING.lock().unwrap(); if b.contains(&id) { return Err("Already downloading.".into()); } b.push(id.clone()); }
    let dir = install_dir(&app)?;
    STOP_DOWNLOAD.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let r = download::install(pack, &dir, &STOP_DOWNLOAD, |done, total| {
            let _ = app.emit("model-progress", DownloadEvent { id: id.clone(), done, total, finished: false, error: None });
        });
        DOWNLOADING.lock().unwrap().retain(|x| x != &id);
        reload_models();
        let _ = app.emit("model-progress", DownloadEvent { id, done: 0, total: 0, finished: true, error: r.err().map(err) });
    });
    Ok(())
}

#[tauri::command]
fn cancel_download() { STOP_DOWNLOAD.store(true, Ordering::SeqCst); }

/// Copies model files the user already has (for example BirdNET's) into the downloads folder.
#[tauri::command]
fn add_models_folder(app: tauri::AppHandle, folder: String) -> R<Vec<&'static str>> {
    let r = download::add_from_folder(Path::new(&folder), &install_dir(&app)?).map_err(err)?;
    reload_models();
    Ok(r)
}

fn path(s: &State<App>) -> R<PathBuf> { s.project.lock().unwrap().clone().ok_or_else(|| "No project is open.".to_string()) }
fn open(s: &State<App>) -> R<Project> { Project::open(&path(s)?).map_err(err) }

#[derive(Serialize)]
struct DepOut { #[serde(flatten)] d: wlnw_engine::store::Deployment, models: Vec<String>, perch_region: bool }

#[derive(Serialize)]
struct Summary {
    path: String, name: String, deployments: Vec<DepOut>, progress: run::Progress, running: bool,
    dataset: export::DatasetInfo, models: Vec<ModelInfo>, model_errors: Vec<String>, chosen: Vec<String>, wnc: Option<WncInfo>, space: run::Space,
    /// Camera-trap images in the project (None when there are none).
    images: Option<images::Counts>,
}

#[derive(Serialize)]
struct WncInfo { trained_at: String, classes: Vec<(String, String, usize, Option<f64>)> }

#[tauri::command]
fn open_project(file: String, s: State<App>) -> R<()> {
    let p = PathBuf::from(&file);
    Project::open(&p).map_err(err)?;
    *s.project.lock().unwrap() = Some(p);
    Ok(())
}

#[tauri::command]
fn summary(app: tauri::AppHandle, s: State<App>) -> R<Summary> {
    let p = open(&s)?;
    let file = path(&s)?;
    let m = models(&app);
    let (ms, errors) = (&m.0, &m.1);
    let have = have(&app);
    let plan = run::plan(&p, &have).map_err(err)?;
    Ok(Summary {
        name: file.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        path: file.to_string_lossy().to_string(),
        deployments: p.deployments().map_err(err)?.into_iter().map(|d| {
            let perch_region = matches!((d.lat, d.lon), (Some(a), Some(b)) if models::perch_first(a, b));
            DepOut { models: plan.get(&d.id).cloned().unwrap_or_default(), perch_region, d }
        }).collect(),
        space: run::space(&p, &plan, &ms.iter().filter(|m| m.info().embeds).map(|m| m.info().id.clone()).collect::<Vec<_>>()).map_err(err)?,
        progress: run::progress_all(&p, &plan, &have).map_err(err)?, running: s.running.load(Ordering::Relaxed),
        images: images::counts(&p, camtrap::ID).ok().filter(|c| c.total > 0),
        dataset: export::info(&p),
        models: ms.iter().map(|m| m.info().clone()).collect(), model_errors: errors.clone(), chosen: p.chosen_models(),
        wnc: wnc::pack(&p).map(|k| WncInfo { trained_at: k.trained_at, classes: k.classes.into_iter().map(|c| (c.scientific, c.common, c.positives, c.cv_accuracy)).collect() }),
    })
}

#[tauri::command]
async fn scan_folder(folder: String, s: State<'_, App>) -> R<scan::ScanReport> {
    let file = path(&s)?;
    tauri::async_runtime::spawn_blocking(move || {
        let p = Project::open(&file).map_err(err)?;
        let t = history::now(&p);
        let r = scan::scan(&p, Path::new(&folder));
        history::log(&p, "scan", &t, &r, |x| scan::summary(x, &folder), "Adding a folder failed");
        r.map_err(err)
    }).await.map_err(err)?
}

#[tauri::command]
fn update_deployment(id: i64, name: String, lat: Option<f64>, lon: Option<f64>, tz_hours: f64, s: State<App>) -> R<()> {
    if lat.is_some_and(|v| !(-90.0..=90.0).contains(&v)) || lon.is_some_and(|v| !(-180.0..=180.0).contains(&v)) { return Err("Latitude must be -90 to 90 and longitude -180 to 180.".into()); }
    open(&s)?.update_deployment(id, &name, lat, lon, (tz_hours * 60.0).round() as i32).map_err(err)
}

/// Camera traps: the deployment's country (ISO 3166-1 alpha-3) and US state for SpeciesNet's geofencing. True when its
/// images will run again.
#[tauri::command]
fn set_deployment_country(id: i64, country: Option<String>, admin1: Option<String>, s: State<App>) -> R<bool> {
    if s.running.load(Ordering::SeqCst) { return Err("Pause processing first.".into()); }
    images::set_country(&open(&s)?, id, country.as_deref(), admin1.as_deref()).map_err(err)
}

#[tauri::command]
fn image_species(s: State<App>) -> R<Vec<images::SpeciesRow>> { images::species(&open(&s)?).map_err(err) }

#[tauri::command]
fn image_queue(query: images::Query, s: State<App>) -> R<Vec<images::ImageItem>> { images::queue(&open(&s)?, &query).map_err(err) }

#[tauri::command]
fn image_filters(s: State<App>) -> R<std::collections::BTreeMap<&'static str, i64>> { images::filter_counts(&open(&s)?).map_err(err) }

#[tauri::command]
fn set_image_verdict(box_id: i64, verdict: Option<String>, s: State<App>) -> R<()> { images::set_verdict(&open(&s)?, box_id, verdict.as_deref()).map_err(err) }

/// A JPEG of an image for the review grid, as base64.
#[tauri::command]
async fn image_thumb(file: String, max: u32) -> R<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let j = images::thumbnail(Path::new(&file), max.clamp(64, 2400)).map_err(err)?;
        Ok(base64::engine::general_purpose::STANDARD.encode(j))
    }).await.map_err(err)?
}

#[tauri::command]
fn set_setting(key: String, value: String, s: State<App>) -> R<()> {
    if !["default_threshold", "dataset", "models"].contains(&key.as_str()) { return Err("unknown setting".into()); }
    open(&s)?.set_setting(&key, &value).map_err(err)
}

#[tauri::command]
fn start_run(app: tauri::AppHandle, s: State<App>) -> R<()> {
    let m = models(&app);
    if m.0.is_empty() { return Err(m.1.join(" ")); }
    if s.running.swap(true, Ordering::SeqCst) { return Err("Already running.".into()); }
    let file = path(&s)?;
    let (stop, running) = (s.stop.clone(), s.running.clone());
    stop.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let ms = models(&app).0.clone();
        // A panic (for example in a decoder on a damaged file) must not leave the app thinking a run is still going.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Project::open(&file).map_err(err).and_then(|p| run::run(&p, &ms, &stop, |pr| { let _ = app.emit("run-progress", pr); }).map_err(err))
        })).unwrap_or_else(|_| Err("Processing stopped unexpectedly on a recording. Resume to carry on; if it happens again, see Files for the recording that failed.".into()));
        running.store(false, Ordering::SeqCst);
        let _ = app.emit("run-done", result.err());
    });
    Ok(())
}

#[tauri::command]
fn stop_run(s: State<App>) { s.stop.store(true, Ordering::SeqCst); }

#[tauri::command]
fn species(s: State<App>) -> R<Vec<review::SpeciesRow>> { review::species(&open(&s)?).map_err(err) }

#[tauri::command]
fn review_queue(label: i64, model: String, n: usize, s: State<App>) -> R<Vec<wlnw_engine::store::Detection>> { review::queue(&open(&s)?, label, &model, n).map_err(err) }

#[derive(Serialize)]
struct ClipOut { wav: String, spec: String, width: usize, height: usize, offset_s: f64, window_s: f64, total_s: f64, max_khz: f64, playback: f64, slowed: f64 }

/// `model`: the detection's model, so bat detections get an ultrasonic, slowed-down clip.
#[tauri::command]
async fn clip(file: String, start_s: f64, end_s: f64, model: Option<String>) -> R<ClipOut> {
    tauri::async_runtime::spawn_blocking(move || {
        let c = review::clip_for(Path::new(&file), start_s, end_s, model.as_deref()).map_err(err)?;
        let b = base64::engine::general_purpose::STANDARD;
        Ok(ClipOut { wav: b.encode(&c.wav), spec: b.encode(&c.spec), width: c.width, height: c.height, offset_s: c.offset_s, window_s: c.window_s, total_s: c.total_s, max_khz: c.max_khz,
            playback: c.playback, slowed: c.slowed })
    }).await.map_err(err)?
}

#[tauri::command]
fn set_verdict(detection: i64, verdict: Option<String>, s: State<App>) -> R<()> { review::set_verdict(&open(&s)?, detection, verdict.as_deref()).map_err(err) }

/// "This clip is species X" (or "X is not here"): an example for the Wildlife Network Classifier. Returns false when the
/// recording has not been through Perch yet (examples need its embedding).
#[tauri::command]
fn name_sound(detection: i64, scientific: String, common: String, present: bool, s: State<App>) -> R<bool> {
    let p = open(&s)?;
    match wnc::window_for(&p, detection).map_err(err)? {
        Some(w) => { wnc::add_example(&p, w, &scientific, &common, present).map_err(err)?; Ok(true) }
        None => Ok(false),
    }
}

#[derive(Serialize)]
struct SimilarOut { window_id: i64, file_id: i64, path: String, start_s: f64, end_s: f64, similarity: f32, site: String }

#[tauri::command]
async fn similar(detection: i64, n: usize, s: State<'_, App>) -> R<Vec<SimilarOut>> {
    let file = path(&s)?;
    tauri::async_runtime::spawn_blocking(move || {
        let p = Project::open(&file).map_err(err)?;
        let w = wnc::window_for(&p, detection).map_err(err)?.ok_or("This recording has not been through Perch yet, so similar sounds cannot be searched.")?;
        Ok(wnc::similar(&p, w, n).map_err(err)?.into_iter().map(|x| SimilarOut { window_id: x.window_id, file_id: x.file_id, path: x.path, start_s: x.start_s, end_s: x.end_s, similarity: x.similarity, site: x.site }).collect())
    }).await.map_err(err)?
}

#[tauri::command]
fn label_window(window: i64, scientific: String, common: String, present: bool, s: State<App>) -> R<()> {
    wnc::add_example(&open(&s)?, window, &scientific, &common, present).map_err(err)
}

#[tauri::command]
async fn train_wnc(s: State<'_, App>) -> R<wnc::TrainReport> {
    let file = path(&s)?;
    tauri::async_runtime::spawn_blocking(move || {
        let p = Project::open(&file).map_err(err)?;
        let t = history::now(&p);
        let r = wnc::train(&p);
        history::log(&p, "train", &t, &r, |x| format!("Trained the Wildlife Network Classifier: {} species, {} detections scored", x.classes.len(), history::n(x.detections as i64)), "Training failed");
        r.map_err(err)
    }).await.map_err(err)?
}

#[tauri::command]
fn wnc_export(out: String, s: State<App>) -> R<()> {
    let p = open(&s)?;
    let k = wnc::pack(&p).ok_or("Nothing trained yet.")?;
    std::fs::write(out, serde_json::to_string_pretty(&k).map_err(err)?).map_err(err)
}

#[tauri::command]
async fn wnc_import(file: String, s: State<'_, App>) -> R<usize> {
    let proj = path(&s)?;
    tauri::async_runtime::spawn_blocking(move || {
        let p = Project::open(&proj).map_err(err)?;
        wnc::import(&p, &std::fs::read_to_string(&file).map_err(err)?).map_err(err)
    }).await.map_err(err)?
}

#[tauri::command]
async fn export_to(format: String, out: String, s: State<'_, App>) -> R<usize> {
    let file = path(&s)?;
    tauri::async_runtime::spawn_blocking(move || {
        let p = Project::open(&file).map_err(err)?;
        let o = Path::new(&out);
        let t = history::now(&p);
        let (r, name) = match format.as_str() {
            "csv" => (export::csv(&p, o), "CSV"), "wdx" => (export::wdx(&p, o), "WDX"), "dwca" => (export::dwca(&p, o), "a Darwin Core Archive"),
            "raven" => (export::raven_audacity(&p, o), "Raven and Audacity tables"), "kaleidoscope" => (export::kaleidoscope(&p, o), "a Kaleidoscope id.csv"),
            "camtrapdp" => (images::camtrapdp(&p, o).map(|r| r.observations), "a Camtrap DP package"),
            _ => return Err("unknown format".into()),
        };
        history::log(&p, "export", &t, &r, |k| format!("Exported {} detections as {name} to {out}", history::n(*k as i64)), "Export failed");
        r.map_err(err)
    }).await.map_err(err)?
}

#[tauri::command]
async fn publish_to(app: tauri::AppHandle, dataset: String, key: String, submit: bool, s: State<'_, App>) -> R<publish::PublishReport> {
    let file = path(&s)?;
    tauri::async_runtime::spawn_blocking(move || {
        let p = Project::open(&file).map_err(err)?;
        let t = history::now(&p);
        let r = publish::publish(&p, dataset.trim(), key.trim(), submit, |d, t| { let _ = app.emit("publish-progress", (d, t)); });
        history::log(&p, "publish", &t, &r, |x| format!("Sent {} records to Wildlife Network dataset {}: {} added, {} already there{}", history::n(x.sent as i64), dataset.trim(),
            history::n(x.added), history::n(x.duplicates), if submit { ", submitted for review" } else { "" }), "Publishing failed");
        r.map_err(err)
    }).await.map_err(err)?
}

#[tauri::command]
fn list_files(app: tauri::AppHandle, query: files::Query, s: State<App>) -> R<files::Page> { files::list(&open(&s)?, &have(&app), &query).map_err(err) }

#[tauri::command]
async fn export_file_list(app: tauri::AppHandle, query: files::Query, out: String, s: State<'_, App>) -> R<usize> {
    let file = path(&s)?;
    let have = have(&app);
    tauri::async_runtime::spawn_blocking(move || files::csv(&Project::open(&file).map_err(err)?, &have, &query, Path::new(&out)).map_err(err)).await.map_err(err)?
}

#[tauri::command]
fn retry_failed(file: Option<i64>, s: State<App>) -> R<usize> {
    if s.running.load(Ordering::SeqCst) { return Err("Pause processing first.".into()); }
    files::retry_failed(&open(&s)?, file).map_err(err)
}

/// Results from another tool (a table or a folder of them), matched to the recordings already in the project.
#[tauri::command]
async fn import_results(app: tauri::AppHandle, path: String, s: State<'_, App>) -> R<import::ImportReport> {
    let file = crate::path(&s)?;
    // Installed models' labels turn common names (Raven tables carry no scientific name) into scientific names.
    let labels: Vec<models::Label> = models(&app).0.iter().flat_map(|m| m.labels().to_vec()).collect();
    tauri::async_runtime::spawn_blocking(move || {
        let p = Project::open(&file).map_err(err)?;
        let t = history::now(&p);
        let r = import::import(&p, Path::new(&path), &labels);
        history::log(&p, "import", &t, &r, |x| import::summary(x, &path), "Import failed");
        r.map_err(err)
    }).await.map_err(err)?
}

#[tauri::command]
fn answers_export(out: String, reviewer: Option<String>, s: State<App>) -> R<usize> {
    let p = open(&s)?;
    let t = history::now(&p);
    let r = agreement::export(&p, Path::new(&out), reviewer.as_deref());
    history::log(&p, "agreement", &t, &r, |n| format!("Saved {} review answers for a second reviewer to {out}", history::n(*n as i64)), "Saving answers failed");
    r.map_err(err)
}

#[tauri::command]
fn answers_load(file: String, s: State<App>) -> R<agreement::LoadReport> {
    let p = open(&s)?;
    let t = history::now(&p);
    let r = agreement::load(&p, Path::new(&file), None);
    history::log(&p, "agreement", &t, &r, |x| format!("Loaded {} answers from {}: {} matched to detections here, {} not found", history::n(x.answers as i64), x.reviewer,
        history::n(x.matched as i64), history::n(x.unmatched as i64)), "Loading answers failed");
    r.map_err(err)
}

#[tauri::command]
fn agreement_report(reviewer: Option<String>, s: State<App>) -> R<agreement::Report> { agreement::report(&open(&s)?, reviewer.as_deref()).map_err(err) }

#[tauri::command]
fn agreement_settle(reviewer: String, detection: i64, verdict: String, s: State<App>) -> R<()> { agreement::settle(&open(&s)?, &reviewer, detection, &verdict).map_err(err) }

#[tauri::command]
fn reviewer_name(s: State<App>) -> R<Option<String>> { open(&s)?.setting("reviewer").map_err(err) }

#[tauri::command]
fn history_list(s: State<App>) -> R<Vec<history::Entry>> { history::list(&open(&s)?, 500).map_err(err) }

// ---------- Wildlife Network when online (app setting, not per project) ----------

/// "Use Wildlife Network when online": on unless turned off. Kept in the app's config folder as settings.json.
static ONLINE: AtomicBool = AtomicBool::new(true);
static PREFETCHING: AtomicBool = AtomicBool::new(false);

fn app_settings_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;
    app.path().app_config_dir().ok().map(|d| d.join("settings.json"))
}
fn load_app_settings(app: &tauri::AppHandle) {
    let v: serde_json::Value = app_settings_path(app).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    ONLINE.store(v["online"].as_bool().unwrap_or(true), Ordering::SeqCst);
}

#[tauri::command]
fn online_enabled() -> bool { ONLINE.load(Ordering::SeqCst) }

#[tauri::command]
fn set_online_enabled(app: tauri::AppHandle, on: bool) -> R<()> {
    ONLINE.store(on, Ordering::SeqCst);
    let p = app_settings_path(&app).ok_or("no settings folder")?;
    if let Some(d) = p.parent() { std::fs::create_dir_all(d).map_err(err)?; }
    let mut v: serde_json::Value = std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| serde_json::json!({}));
    v["online"] = serde_json::Value::Bool(on);
    std::fs::write(&p, v.to_string()).map_err(err)
}

/// Fills the project's cache from wlnw.org in the background (recorder positions rounded to about 10 km, species names);
/// emits "online-updated" as answers arrive. Does nothing when the setting is off or a fill is already running.
#[tauri::command]
fn online_prefetch(app: tauri::AppHandle, s: State<App>) -> R<()> {
    if !ONLINE.load(Ordering::SeqCst) { return Ok(()); }
    let file = path(&s)?;
    if PREFETCHING.swap(true, Ordering::SeqCst) { return Ok(()); }
    std::thread::spawn(move || {
        let r = Project::open(&file).ok().map(|p| online::prefetch(&p, &online::http_json, chrono::Utc::now(), || { let _ = app.emit("online-updated", ()); }));
        PREFETCHING.store(false, Ordering::SeqCst);
        if matches!(r, Some(Ok(n)) if n > 0) { let _ = app.emit("online-updated", ()); }
    });
    Ok(())
}

/// Species-list markers from the cache (no network): label id to "unusual" or "few".
#[tauri::command]
fn online_flags(s: State<App>) -> R<std::collections::HashMap<i64, online::Status>> {
    if !ONLINE.load(Ordering::SeqCst) { return Ok(Default::default()); }
    online::flags(&open(&s)?, chrono::Utc::now()).map_err(err)
}

/// The Review box for one species at one recorder; None when the setting is off.
#[tauri::command]
async fn online_species(scientific: String, model: String, deployment: i64, s: State<'_, App>) -> R<Option<online::SpeciesInfo>> {
    if !ONLINE.load(Ordering::SeqCst) { return Ok(None); }
    let file = path(&s)?;
    tauri::async_runtime::spawn_blocking(move || {
        let p = Project::open(&file).map_err(err)?;
        let d = p.deployments().map_err(err)?.into_iter().find(|d| d.id == deployment);
        let (lat, lon) = d.map(|d| (d.lat, d.lon)).unwrap_or((None, None));
        let net = |u: &str| online::http_json(u);
        Ok(Some(online::species_info(&p, &scientific, &model, lat, lon, Some(&net), chrono::Utc::now())))
    }).await.map_err(err)?
}

/// "Around this recorder" for every recorder with a position, from the cache (the background fill brings new answers).
#[tauri::command]
fn online_around(s: State<App>) -> R<Vec<online::Around>> {
    if !ONLINE.load(Ordering::SeqCst) { return Ok(vec![]); }
    let p = open(&s)?;
    let now = chrono::Utc::now();
    Ok(p.deployments().map_err(err)?.into_iter().filter_map(|d| Some(online::around(&p, d.id, d.lat?, d.lon?, None, now))).collect())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            wlnw_engine::birdnet::init_threads();
            // Updates come from the latest GitHub release (latest.json), checked by the UI on startup.
            #[cfg(desktop)]
            {
                app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;
                app.handle().plugin(tauri_plugin_process::init())?;
            }
            load_app_settings(app.handle());
            Ok(())
        })
        .manage({
            // A project file given at launch (double-click or `open --args`) opens straight away.
            let a = App::default();
            if let Some(f) = std::env::args().skip(1).find(|x| x.ends_with(".wlnw") && Path::new(x).exists()) { *a.project.lock().unwrap() = Some(PathBuf::from(f)); }
            a
        })
        .invoke_handler(tauri::generate_handler![open_project, summary, scan_folder, update_deployment, set_setting, start_run, stop_run,
            species, review_queue, clip, set_verdict, model_catalog, download_model, cancel_download, add_models_folder, name_sound, similar, label_window, train_wnc, wnc_export, wnc_import, export_to, publish_to,
            list_files, export_file_list, retry_failed, history_list, import_results, answers_export, answers_load, agreement_report, agreement_settle, reviewer_name,
            online_enabled, set_online_enabled, online_prefetch, online_flags, online_species, online_around,
            set_deployment_country, image_species, image_queue, image_filters, set_image_verdict, image_thumb])
        .run(tauri::generate_context!())
        .expect("error while running Wildlife Network Studio");
}
