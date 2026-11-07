//! `wlnw`: the Studio engine from the command line, for servers, scripts and tests.
//!   wlnw models [models dir]                       list the model packs found
//!   wlnw download <pack id> [models dir]           download a model pack (perch-2.0)
//!   wlnw scan <project.wlnw> <folder>
//!   wlnw place <project.wlnw> <deployment id> <lat> <lon> <utc offset hours>
//!   wlnw use <project.wlnw> [model ids...]          models to run (none = regional default)
//!   wlnw run <project.wlnw> [models dir]
//!   wlnw status <project.wlnw> [models dir]
//!   wlnw species <project.wlnw>
//!   wlnw verdict <project.wlnw> <detection id> yes|no|unsure
//!   wlnw example <project.wlnw> <detection id> <scientific name> [absent]
//!   wlnw similar <project.wlnw> <detection id> [n]
//!   wlnw train <project.wlnw>                       train the Wildlife Network Classifier
//!   wlnw wnc-export <project.wlnw> <out.wnc> | wlnw wnc-import <project.wlnw> <file.wnc>
//!   wlnw export <project.wlnw> csv|wdx|dwca|raven <out>
//!   wlnw publish <project.wlnw> <dataset id> [--submit]   (key in WLNW_UPLOAD_KEY)
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use wlnw_engine::{export, models, publish, review, run, scan, store::Project, wnc};

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
        "scan" => println!("{}", serde_json::to_string_pretty(&scan::scan(&proj()?, Path::new(a.get(3).context("folder?")?))?)?),
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
        "train" => println!("{}", serde_json::to_string_pretty(&wnc::train(&proj()?)?)?),
        "wnc-export" => { let p = proj()?; std::fs::write(&a[3], serde_json::to_string_pretty(&wnc::pack(&p).context("nothing trained yet")?)?)?; }
        "wnc-import" => println!("{} detections", wnc::import(&proj()?, &std::fs::read_to_string(&a[3])?)?),
        "export" => {
            let p = proj()?;
            let out = Path::new(a.get(4).context("output path?")?);
            let n = match a.get(3).map(String::as_str) {
                Some("csv") => export::csv(&p, out)?,
                Some("wdx") => export::wdx(&p, out)?,
                Some("dwca") => export::dwca(&p, out)?,
                Some("raven") => export::raven_audacity(&p, out)?,
                _ => bail!("format: csv, wdx, dwca or raven"),
            };
            println!("{n} detections exported to {}", out.display());
        }
        "publish" => {
            let key = std::env::var("WLNW_UPLOAD_KEY").context("set WLNW_UPLOAD_KEY to the dataset's upload key")?;
            let r = publish::publish(&proj()?, a.get(3).context("dataset id?")?, &key, a.iter().any(|x| x == "--submit"), |d, t| eprintln!("{d}/{t}"))?;
            println!("{}", serde_json::to_string_pretty(&r)?);
        }
        _ => println!("{}", include_str!("wlnw.rs").lines().skip(1).take_while(|l| l.starts_with("//!")).map(|l| l.trim_start_matches("//!")).collect::<Vec<_>>().join("\n")),
    }
    Ok(())
}
