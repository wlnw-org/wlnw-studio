//! `wlnw`: the Studio engine from the command line, for servers, scripts and tests.
//!   wlnw models [models dir]                       list the model packs found
//!   wlnw download <pack id> [models dir]           download a model pack (perch-2.0)
//!   wlnw scan <project.wlnw> <folder>
//!   wlnw place <project.wlnw> <deployment id> <lat> <lon> <utc offset hours>
//!   wlnw use <project.wlnw> [model ids...]          models to run (none = regional default)
//!   wlnw run <project.wlnw> [models dir]
//!   wlnw status <project.wlnw> [models dir]
//!   wlnw files <project.wlnw> [done|partial|todo|failed|none] [models dir]
//!   wlnw retry <project.wlnw>                       try failed recordings again on the next run
//!   wlnw history <project.wlnw>
//!   wlnw species <project.wlnw>
//!   wlnw verdict <project.wlnw> <detection id> yes|no|unsure
//!   wlnw example <project.wlnw> <detection id> <scientific name> [absent]
//!   wlnw similar <project.wlnw> <detection id> [n]
//!   wlnw train <project.wlnw>                       train the Wildlife Network Classifier
//!   wlnw wnc-export <project.wlnw> <out.wnc> | wlnw wnc-import <project.wlnw> <file.wnc>
//!   wlnw import <project.wlnw> <file or folder>     results from BirdNET-Analyzer, BirdNET-Pi/Go or Kaleidoscope
//!   wlnw answers-export <project.wlnw> <out.wlnr> [your name]   your review answers, for a second reviewer
//!   wlnw answers-load <project.wlnw> <file.wlnr> [label]        another reviewer's answers
//!   wlnw agreement <project.wlnw> [reviewer]         agreement per species and disagreements
//!   wlnw settle <project.wlnw> <reviewer> <detection id> yes|no|unsure
//!   wlnw export <project.wlnw> csv|wdx|dwca|raven|kaleidoscope <out>
//!   wlnw publish <project.wlnw> <dataset id> [--submit]   (key in WLNW_UPLOAD_KEY)
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use wlnw_engine::{agreement, export, import, files, history, models, publish, review, run, scan, store::Project, wnc};

fn models_dir(dir: Option<&String>) -> PathBuf {
    dir.map(PathBuf::from).or_else(|| std::env::var("WLNW_MODELS").ok().map(PathBuf::from)).unwrap_or_else(|| PathBuf::from("models"))
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let cmd = a.get(1).map(String::as_str).unwrap_or("help");
    let proj = || -> Result<Project> { Project::open(Path::new(a.get(2).context("project file?")?)) };
    match cmd {
        "models" => {
            let (ms, errs) = models::available(&models_dir(a.get(2)));
            for m in &ms { let i = m.info(); println!("{:<12} {} {}  {} labels, {} kHz, {:.0} s windows, licence {}", i.id, i.name, i.version, m.labels().len(), i.sample_rate / 1000, i.window as f64 / i.sample_rate as f64, i.licence); }
            for e in errs { eprintln!("could not load {e}"); }
        }
        "download" => {
            let pack = wlnw_engine::download::CATALOG.iter().find(|p| Some(p.id) == a.get(2).map(String::as_str)).context("pack id? (perch-2.0)")?;
            let stop = AtomicBool::new(false);
            let mut last = 0;
            wlnw_engine::download::install(pack, &models_dir(a.get(3)), &stop, |d, t| { let pc = d * 100 / t.max(1); if pc >= last + 10 || d == t { last = pc; eprintln!("{pc}%"); } })?;
            println!("{} installed", pack.name);
        }
        "scan" => {
            let p = proj()?;
            let folder = a.get(3).context("folder?")?;
            let t = history::now(&p);
            let r = scan::scan(&p, Path::new(folder));
            history::log(&p, "scan", &t, &r, |x| format!("Added {} new of {} recordings found in {folder} (command line)", history::n(x.added as i64), history::n(x.found as i64)), "Adding a folder failed");
            println!("{}", serde_json::to_string_pretty(&r?)?);
        }
        "place" => {
            let p = proj()?;
            let id: i64 = a[3].parse()?;
            let d = p.deployments()?.into_iter().find(|d| d.id == id).context("no such deployment")?;
            p.update_deployment(id, &d.name, Some(a[4].parse()?), Some(a[5].parse()?), (a[6].parse::<f64>()? * 60.0) as i32)?;
        }
        "use" => proj()?.set_setting("models", &serde_json::to_string(&a[3..].to_vec())?)?,
        "status" => {
            let p = proj()?;
            let (ms, _) = models::available(&models_dir(a.get(3)));
            let have: Vec<String> = ms.iter().map(|m| m.info().id.clone()).collect();
            let plan = run::plan(&p, &have)?;
            println!("{}", serde_json::to_string_pretty(&run::progress(&p, &plan)?)?);
            let emb: Vec<String> = ms.iter().filter(|m| m.info().embeds).map(|m| m.info().id.clone()).collect();
            let sp = run::space(&p, &plan, &emb)?;
            let size = |b: u64| if b >= 1_000_000_000 { format!("{:.1} GB", b as f64 / 1e9) } else { format!("{} MB", (b as f64 / 1e6).round()) };
            eprintln!("disk: about {} still needed for embeddings, {} free{}", size(sp.needed),
                sp.free.map(size).unwrap_or_else(|| "unknown".into()), if sp.enough { "" } else { "  NOT ENOUGH" });
            for d in p.deployments()? { println!("{:>4} {:<24} {:>5} files {:>7.1} h  lat {:?} lon {:?} utc{:+.1}  models {:?}", d.id, d.name, d.files, d.hours, d.lat, d.lon, d.tz_offset_min as f64 / 60.0, plan.get(&d.id)); }
        }
        "run" => {
            let p = proj()?;
            let (ms, errs) = models::available(&models_dir(a.get(3)));
            for e in errs { eprintln!("could not load {e}"); }
            let stop = AtomicBool::new(false);
            let r = run::run(&p, &ms, &stop, |pr| eprintln!("{}/{} jobs, {} detections{}", pr.jobs_done, pr.jobs_total, pr.detections,
                pr.current.as_deref().map(|c| format!(", now {} on {c}", pr.current_model.as_deref().unwrap_or(""))).unwrap_or_default()))?;
            println!("{}", serde_json::to_string_pretty(&r)?);
        }
        "files" => {
            let p = proj()?;
            let (ms, _) = models::available(&models_dir(a.get(4)));
            let have: Vec<String> = ms.iter().map(|m| m.info().id.clone()).collect();
            let q = files::Query { status: a.get(3).cloned(), limit: 2000, ..Default::default() };
            let page = files::list(&p, &have, &q)?;
            for r in &page.rows {
                let errs = r.models.iter().filter_map(|m| m.error.as_ref().map(|e| format!("  {}: {e}", m.model))).collect::<Vec<_>>().join("");
                println!("{:<8} {:>6} det  {}{errs}", r.status, r.detections, r.path);
            }
            let c = page.counts;
            eprintln!("{} recordings: {} done, {} partly done, {} to do, {} failed, {} without a model", c.all, c.done, c.partial, c.todo, c.failed, c.none);
        }
        "retry" => println!("{} failed attempts cleared", files::retry_failed(&proj()?, None)?),
        "history" => {
            for e in history::list(&proj()?, 500)? { println!("{}  {:<8} {:<11} {}", e.started_at, e.kind, e.status, e.summary); }
        }
        "species" => {
            for s in review::species(&proj()?)? { println!("{:>6} {:>6}  {:<12} t={:.2} ({})  {} / {}", s.detections, s.kept, s.model, s.threshold, s.threshold_source, s.common, s.scientific); }
        }
        "verdict" => review::set_verdict(&proj()?, a[3].parse()?, Some(a[4].as_str()))?,
        "example" => {
            let p = proj()?;
            let w = wnc::window_for(&p, a[3].parse()?)?.context("that detection has no Perch window: run Perch on its recording first")?;
            wnc::add_example(&p, w, &a[4], "", a.get(5).map(String::as_str) != Some("absent"))?;
        }
        "similar" => {
            let p = proj()?;
            let w = wnc::window_for(&p, a[3].parse()?)?.context("no Perch window for that detection")?;
            for s in wnc::similar(&p, w, a.get(4).and_then(|n| n.parse().ok()).unwrap_or(10))? { println!("{:.3}  {}  {:.0} s  window {}", s.similarity, s.path, s.start_s, s.window_id); }
        }
        "train" => {
            let p = proj()?;
            let t = history::now(&p);
            let r = wnc::train(&p);
            history::log(&p, "train", &t, &r, |x| format!("Trained the Wildlife Network Classifier: {} species, {} detections scored (command line)", x.classes.len(), history::n(x.detections as i64)), "Training failed");
            println!("{}", serde_json::to_string_pretty(&r?)?);
        }
        "wnc-export" => { let p = proj()?; std::fs::write(&a[3], serde_json::to_string_pretty(&wnc::pack(&p).context("nothing trained yet")?)?)?; }
        "wnc-import" => println!("{} detections", wnc::import(&proj()?, &std::fs::read_to_string(&a[3])?)?),
        "export" => {
            let p = proj()?;
            let out = Path::new(a.get(4).context("output path?")?);
            let t = history::now(&p);
            let (r, name) = match a.get(3).map(String::as_str) {
                Some("csv") => (export::csv(&p, out), "CSV"),
                Some("wdx") => (export::wdx(&p, out), "WDX"),
                Some("dwca") => (export::dwca(&p, out), "a Darwin Core Archive"),
                Some("raven") => (export::raven_audacity(&p, out), "Raven and Audacity tables"),
                Some("kaleidoscope") => (export::kaleidoscope(&p, out), "a Kaleidoscope id.csv"),
                _ => bail!("format: csv, wdx, dwca, raven or kaleidoscope"),
            };
            history::log(&p, "export", &t, &r, |k| format!("Exported {} detections as {name} to {} (command line)", history::n(*k as i64), out.display()), "Export failed");
            let n = r?;
            println!("{n} detections exported to {}", out.display());
        }
        "import" => {
            let p = proj()?;
            let src = a.get(3).context("results file or folder?")?;
            let t = history::now(&p);
            // Installed models' labels turn common names (Raven tables) into scientific names.
            let (ms, _) = models::available(&models_dir(None));
            let labels: Vec<models::Label> = ms.iter().flat_map(|m| m.labels().to_vec()).collect();
            let r = import::import(&p, Path::new(src), &labels);
            history::log(&p, "import", &t, &r, |x| import::summary(x, src) + " (command line)", "Import failed");
            let r = r?;
            println!("{}", serde_json::to_string_pretty(&r)?);
        }
        "answers-export" => {
            let p = proj()?;
            let out = a.get(3).context("output .wlnr?")?;
            let t = history::now(&p);
            let r = agreement::export(&p, Path::new(out), a.get(4).map(String::as_str));
            history::log(&p, "agreement", &t, &r, |n| format!("Saved {} review answers for a second reviewer to {out} (command line)", history::n(*n as i64)), "Saving answers failed");
            println!("{} answers saved to {out}", r?);
        }
        "answers-load" => {
            let p = proj()?;
            let f = a.get(3).context(".wlnr file?")?;
            let t = history::now(&p);
            let r = agreement::load(&p, Path::new(f), a.get(4).map(String::as_str));
            history::log(&p, "agreement", &t, &r, |x| format!("Loaded {} answers from {}: {} matched, {} not found (command line)", history::n(x.answers as i64), x.reviewer, history::n(x.matched as i64), history::n(x.unmatched as i64)), "Loading answers failed");
            println!("{}", serde_json::to_string_pretty(&r?)?);
        }
        "agreement" => {
            let r = agreement::report(&proj()?, a.get(3).map(String::as_str))?;
            let Some(who) = &r.reviewer else { bail!("no second reviewer loaded yet: wlnw answers-load first") };
            let pc = |v: Option<f64>| v.map_or("-".into(), |x| format!("{x:.0}%"));
            let k = |v: Option<f64>| v.map_or("-".into(), |x| format!("{x:.2}"));
            println!("{:<36} {:>5} {:>12} {:>12} {:>6}", format!("compared with {who}"), "both", "yes/no", "agree", "kappa");
            for s in std::iter::once(&r.overall).chain(&r.species) { println!("{:<36} {:>5} {:>12} {:>12} {:>6}", s.scientific, s.both, s.yes_no, pc(s.percent), k(s.kappa)); }
            println!("\n{} disagreements to settle, {} settled", r.disagreements.len(), r.settled);
            for d in &r.disagreements { println!("{:>8}  {}  {:.1} s  {}  you {}, {who} {}", d.detection.id, d.detection.path, d.detection.start_s, d.detection.scientific, d.detection.verdict.as_deref().unwrap_or(""), d.other); }
        }
        "settle" => agreement::settle(&proj()?, a.get(3).context("reviewer?")?, a.get(4).context("detection id?")?.parse()?, a.get(5).context("yes, no or unsure?")?)?,
        "publish" => {
            let key = std::env::var("WLNW_UPLOAD_KEY").context("set WLNW_UPLOAD_KEY to the dataset's upload key")?;
            let p = proj()?;
            let dataset = a.get(3).context("dataset id?")?;
            let submit = a.iter().any(|x| x == "--submit");
            let t = history::now(&p);
            let r = publish::publish(&p, dataset, &key, submit, |d, t| eprintln!("{d}/{t}"));
            history::log(&p, "publish", &t, &r, |x| format!("Sent {} records to Wildlife Network dataset {dataset}: {} added, {} already there{} (command line)", history::n(x.sent as i64),
                history::n(x.added), history::n(x.duplicates), if submit { ", submitted for review" } else { "" }), "Publishing failed");
            println!("{}", serde_json::to_string_pretty(&r?)?);
        }
        _ => println!("{}", include_str!("wlnw.rs").lines().skip(1).take_while(|l| l.starts_with("//!")).map(|l| l.trim_start_matches("//!")).collect::<Vec<_>>().join("\n")),
    }
    Ok(())
}
