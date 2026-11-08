//! Results made by other tools, brought in so a lab can review and publish without processing the audio again:
//! BirdNET-Analyzer (CSV, combined table, Raven selection tables), BirdNET-Pi and BirdNET-Go detection exports, and
//! Kaleidoscope id.csv (bats and birds). Each row is matched to a recording already in the project, by full path, else by
//! file name (with the folder to break ties), else, for BirdNET-Pi/Go whose files are clips, by date and time.
//! Imported rows become ordinary detections under a model id "import:<tool>", so review, thresholds and exports treat
//! them like any model's; they have no embeddings, so similarity search is not offered for them.
use crate::models::Label;
use crate::store::Project;
use anyhow::{bail, Result};
use chrono::{Duration, NaiveDateTime};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

pub const ANALYZER: &str = "import:birdnet-analyzer";
pub const BIRDNET_PI: &str = "import:birdnet-pi";
pub const KALEIDOSCOPE: &str = "import:kaleidoscope";

#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    /// Result rows read (each table's data rows, all tables in a folder).
    pub rows: usize,
    /// Rows matched to a recording in the project, and of those, new detections (a second import adds nothing twice).
    pub matched: usize, pub added: usize,
    /// Rows whose recording is not in the project (or could be more than one), with a few examples.
    pub unmatched: usize, pub unmatched_examples: Vec<String>,
    /// Rows with no species: noise, "NoID", or a name or code Studio cannot turn into a scientific name (examples kept).
    pub no_species: usize, pub unknown_names: Vec<String>,
    /// Tables read, and the model id(s) the detections were stored under.
    pub tables: usize, pub model: String,
}

/// Kaleidoscope's six-letter bat codes (first three letters of genus and species) for North America and Europe.
const BAT_CODES: &[(&str, &str, &str)] = &[
    ("ANTPAL", "Antrozous pallidus", "Pallid bat"), ("CORRAF", "Corynorhinus rafinesquii", "Rafinesque's big-eared bat"),
    ("CORTOW", "Corynorhinus townsendii", "Townsend's big-eared bat"), ("EPTFUS", "Eptesicus fuscus", "Big brown bat"),
    ("EUDMAC", "Euderma maculatum", "Spotted bat"), ("EUMPER", "Eumops perotis", "Western mastiff bat"),
    ("IDIPHY", "Idionycteris phyllotis", "Allen's big-eared bat"), ("LASBOR", "Lasiurus borealis", "Eastern red bat"),
    ("LASCIN", "Lasiurus cinereus", "Hoary bat"), ("LASEGA", "Lasiurus ega", "Southern yellow bat"),
    ("LASINT", "Lasiurus intermedius", "Northern yellow bat"), ("LASSEM", "Lasiurus seminolus", "Seminole bat"),
    ("LASXAN", "Lasiurus xanthinus", "Western yellow bat"), ("LASNOC", "Lasionycteris noctivagans", "Silver-haired bat"),
    ("MYOAUS", "Myotis austroriparius", "Southeastern myotis"), ("MYOCAL", "Myotis californicus", "California myotis"),
    ("MYOCIL", "Myotis ciliolabrum", "Western small-footed myotis"), ("MYOEVO", "Myotis evotis", "Long-eared myotis"),
    ("MYOGRI", "Myotis grisescens", "Gray bat"), ("MYOLEI", "Myotis leibii", "Eastern small-footed myotis"),
    ("MYOLUC", "Myotis lucifugus", "Little brown bat"), ("MYOSEP", "Myotis septentrionalis", "Northern long-eared bat"),
    ("MYOSOD", "Myotis sodalis", "Indiana bat"), ("MYOTHY", "Myotis thysanodes", "Fringed myotis"),
    ("MYOVEL", "Myotis velifer", "Cave myotis"), ("MYOVOL", "Myotis volans", "Long-legged myotis"),
    ("MYOYUM", "Myotis yumanensis", "Yuma myotis"), ("NYCHUM", "Nycticeius humeralis", "Evening bat"),
    ("NYCMAC", "Nyctinomops macrotis", "Big free-tailed bat"), ("NYCFEM", "Nyctinomops femorosaccus", "Pocketed free-tailed bat"),
    ("PARHES", "Parastrellus hesperus", "Canyon bat"), ("PERSUB", "Perimyotis subflavus", "Tricolored bat"),
    ("TADBRA", "Tadarida brasiliensis", "Brazilian free-tailed bat"),
    ("BARBAR", "Barbastella barbastellus", "Western barbastelle"), ("EPTSER", "Eptesicus serotinus", "Serotine"),
    ("EPTNIL", "Eptesicus nilssonii", "Northern bat"), ("HYPSAV", "Hypsugo savii", "Savi's pipistrelle"),
    ("MINSCH", "Miniopterus schreibersii", "Common bent-wing bat"), ("MYOALC", "Myotis alcathoe", "Alcathoe bat"),
    ("MYOBEC", "Myotis bechsteinii", "Bechstein's bat"), ("MYOBLY", "Myotis blythii", "Lesser mouse-eared bat"),
    ("MYOBRA", "Myotis brandtii", "Brandt's bat"), ("MYOCAP", "Myotis capaccinii", "Long-fingered bat"),
    ("MYODAS", "Myotis dasycneme", "Pond bat"), ("MYODAU", "Myotis daubentonii", "Daubenton's bat"),
    ("MYOEMA", "Myotis emarginatus", "Geoffroy's bat"), ("MYOMYO", "Myotis myotis", "Greater mouse-eared bat"),
    ("MYOMYS", "Myotis mystacinus", "Whiskered bat"), ("MYONAT", "Myotis nattereri", "Natterer's bat"),
    ("NYCLAS", "Nyctalus lasiopterus", "Greater noctule"), ("NYCLEI", "Nyctalus leisleri", "Leisler's bat"),
    ("NYCNOC", "Nyctalus noctula", "Common noctule"), ("PIPKUH", "Pipistrellus kuhlii", "Kuhl's pipistrelle"),
    ("PIPNAT", "Pipistrellus nathusii", "Nathusius' pipistrelle"), ("PIPPIP", "Pipistrellus pipistrellus", "Common pipistrelle"),
    ("PIPPYG", "Pipistrellus pygmaeus", "Soprano pipistrelle"), ("PLEAUR", "Plecotus auritus", "Brown long-eared bat"),
    ("PLEAUS", "Plecotus austriacus", "Grey long-eared bat"), ("RHIEUR", "Rhinolophus euryale", "Mediterranean horseshoe bat"),
    ("RHIFER", "Rhinolophus ferrumequinum", "Greater horseshoe bat"), ("RHIHIP", "Rhinolophus hipposideros", "Lesser horseshoe bat"),
    ("TADTEN", "Tadarida teniotis", "European free-tailed bat"), ("VESMUR", "Vespertilio murinus", "Parti-coloured bat"),
];
/// Labels that mean "no species here".
const NOTHING: &[&str] = &["", "noise", "noid", "no id", "nocall", "no call", "unknown", "none", "nospp", "no spp"];

/// A small CSV/TSV reader: quoted fields with "" escapes, separator guessed from the header line.
fn table(text: &str) -> Vec<Vec<String>> {
    let text = text.trim_start_matches('\u{feff}');
    let head = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let sep = if head.contains('\t') { '\t' } else if head.matches(';').count() > head.matches(',').count() { ';' } else { ',' };
    let (mut rows, mut row, mut cell, mut quoted) = (Vec::new(), Vec::new(), String::new(), false);
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' if quoted && it.peek() == Some(&'"') => { cell.push('"'); it.next(); }
            '"' if quoted => quoted = false,
            '"' if cell.is_empty() => quoted = true,
            c if c == sep && !quoted => row.push(std::mem::take(&mut cell)),
            '\r' if !quoted => {}
            '\n' if !quoted => { row.push(std::mem::take(&mut cell)); rows.push(std::mem::take(&mut row)); }
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() { row.push(cell); rows.push(row); }
    rows.retain(|r| r.iter().any(|c| !c.trim().is_empty()));
    rows
}

/// "Begin Time (s)" -> "begintimes", "AUTO ID*" -> "autoid", "Sci_Name" -> "sciname".
fn norm(h: &str) -> String { h.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase() }

#[derive(Debug, Clone, PartialEq)]
struct Row {
    file: Option<String>, folder: Option<String>, start: Option<f64>, end: Option<f64>, when: Option<NaiveDateTime>,
    name: String, common: String, conf: f64, manual: Option<String>,
}

/// Which tool made a table, from its header; None for a table Studio does not know.
fn detect(h: &[String]) -> Option<&'static str> {
    let has = |k: &str| h.iter().any(|x| x == k);
    if has("autoid") || has("manualid") { return Some(KALEIDOSCOPE); }
    let named = ["scientificname", "sciname", "commonname", "comname", "species"].iter().any(|k| has(k));
    if named && (has("starts") || has("begintimes") || has("offset")) { return Some(ANALYZER); }
    if named && has("date") && has("time") { return Some(BIRDNET_PI); }
    None
}

fn conf(v: &str) -> Option<f64> {
    let x: f64 = v.trim().trim_end_matches('%').replace(',', ".").parse().ok()?;
    // BirdNET-Go and some exports write percentages.
    Some(if x > 1.0 { x / 100.0 } else { x }.clamp(0.0, 1.0))
}

fn when(date: &str, time: &str) -> Option<NaiveDateTime> {
    let s = format!("{} {}", date.trim().replace('/', "-"), time.trim());
    ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"].iter().find_map(|f| NaiveDateTime::parse_from_str(&s, f).ok())
}

/// Rows of one table, or None when it is not a results table.
fn parse(text: &str) -> Option<(&'static str, Vec<Row>)> {
    let t = table(text);
    let (head, body) = t.split_first()?;
    let h: Vec<String> = head.iter().map(|x| norm(x)).collect();
    let format = detect(&h)?;
    let col = |keys: &[&str]| keys.iter().find_map(|k| h.iter().position(|x| x == k));
    let (c_file, c_folder) = (col(&["file", "beginpath", "filename", "infile", "filepath", "path"]), col(&["folder"]));
    let (c_off, c_start, c_end, c_dur) = (col(&["fileoffsets"]), col(&["starts", "begintimes", "offset", "start"]), col(&["ends", "endtimes", "end"]), col(&["duration"]));
    let c_name = if format == KALEIDOSCOPE { col(&["autoid"]) } else { col(&["scientificname", "sciname", "species"]) };
    let (c_common, c_conf, c_manual) = (col(&["commonname", "comname"]), col(&["confidence", "matchratio", "conf"]), col(&["manualid"]));
    let (c_date, c_time) = (col(&["date"]), col(&["time"]));
    let rows = body.iter().map(|r| {
        let get = |c: Option<usize>| c.and_then(|i| r.get(i)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let num = |c: Option<usize>| get(c).and_then(|s| s.replace(',', ".").parse::<f64>().ok());
        let (begin, end) = (num(c_start), num(c_end));
        // Combined Raven tables count Begin Time across all files; File Offset is the time within the recording.
        let start = num(c_off).or(begin);
        let len = match (begin, end, num(c_dur)) { (Some(b), Some(e), _) if e > b => e - b, (_, _, Some(d)) if d > 0.0 => d, _ => 3.0 };
        Row {
            file: get(c_file), folder: get(c_folder), start, end: start.map(|s| s + len),
            when: match (get(c_date), get(c_time)) { (Some(d), Some(t)) => when(&d, &t), _ => None },
            name: get(c_name).unwrap_or_default(), common: get(c_common).unwrap_or_default(),
            conf: get(c_conf).and_then(|v| conf(&v)).unwrap_or(1.0), manual: get(c_manual),
        }
    }).collect();
    Some((format, rows))
}

/// Turns a tool's label into (scientific, common): a scientific name as given, a Kaleidoscope bat code, or a common name
/// some model in the project (or the given labels) knows. None for noise and for names that cannot be resolved.
struct Names { common: HashMap<String, (String, String)> }
impl Names {
    fn new(p: &Project, labels: &[Label]) -> Result<Self> {
        let mut common = HashMap::new();
        for l in labels { if !l.common.is_empty() { common.entry(l.common.to_lowercase()).or_insert((l.scientific.clone(), l.common.clone())); } }
        let mut st = p.db.prepare("SELECT scientific, common FROM species WHERE common <> ''")?;
        for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? { let (s, c) = r?; common.insert(c.to_lowercase(), (s, c)); }
        Ok(Names { common })
    }
    fn resolve(&self, name: &str, common: &str) -> Option<(String, String)> {
        let n = name.trim();
        if NOTHING.contains(&n.to_lowercase().as_str()) && NOTHING.contains(&common.trim().to_lowercase().as_str()) { return None; }
        if let Some(b) = BAT_CODES.iter().find(|b| b.0.eq_ignore_ascii_case(n)) { return Some((b.1.into(), b.2.into())); }
        let looks_scientific = n.split_whitespace().count() >= 2 && n.chars().next().is_some_and(|c| c.is_uppercase())
            && n.split_whitespace().nth(1).is_some_and(|w| w.chars().all(|c| c.is_lowercase() || c == '-'));
        if looks_scientific {
            let common = if common.trim().is_empty() { self.common.values().find(|v| v.0 == n).map(|v| v.1.clone()).unwrap_or_default() } else { common.trim().to_string() };
            return Some((n.to_string(), common));
        }
        // Raven tables from BirdNET-Analyzer carry only the common name; Kaleidoscope bird IDs may be common names.
        [common.trim(), n].iter().filter(|s| !s.is_empty()).find_map(|s| self.common.get(&s.to_lowercase()).cloned())
    }
}

/// The project's recordings, indexed for matching rows to them.
struct Recordings {
    by_path: HashMap<String, i64>,
    by_name: HashMap<String, Vec<(i64, String)>>,
    by_stem: HashMap<String, Vec<i64>>,
    /// (file, start, end) in each recorder's local time; for files not yet processed, until the next file starts (at most an hour).
    timeline: Vec<(i64, NaiveDateTime, NaiveDateTime)>,
}
enum Found { One(i64, Option<f64>), Many, Nothing }

fn slashes(s: &str) -> String { s.replace('\\', "/") }
fn base(s: &str) -> String { slashes(s).rsplit('/').next().unwrap_or("").to_lowercase() }

impl Recordings {
    fn new(p: &Project) -> Result<Self> {
        let mut r = Recordings { by_path: HashMap::new(), by_name: HashMap::new(), by_stem: HashMap::new(), timeline: Vec::new() };
        let mut st = p.db.prepare("SELECT id, path, deployment_id, start_local, duration_s FROM files ORDER BY deployment_id, start_local")?;
        let rows: Vec<(i64, String, i64, Option<String>, Option<f64>)> = st.query_map([], |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?, x.get(3)?, x.get(4)?)))?.collect::<Result<_, _>>()?;
        for (k, (id, path, dep, start, dur)) in rows.iter().enumerate() {
            let norm = slashes(path);
            r.by_path.insert(norm.clone(), *id);
            r.by_name.entry(base(path)).or_default().push((*id, norm.to_lowercase()));
            let name = base(path);
            r.by_stem.entry(name.rsplit_once('.').map_or(name.clone(), |x| x.0.to_string())).or_default().push(*id);
            let Some(t) = start.as_deref().and_then(|s| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()) else { continue };
            let next = rows.get(k + 1).filter(|n| n.2 == *dep).and_then(|n| n.3.as_deref()).and_then(|s| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok());
            let end = match dur { Some(d) => t + Duration::milliseconds((d * 1000.0) as i64), None => next.filter(|n| *n > t).unwrap_or(t + Duration::hours(1)).min(t + Duration::hours(1)) };
            r.timeline.push((*id, t, end));
        }
        Ok(r)
    }

    fn by_file(&self, file: &str, folder: Option<&str>) -> Found {
        let f = slashes(file);
        if let Some(&id) = self.by_path.get(&f) { return Found::One(id, None); }
        if let Some(d) = folder { if let Some(&id) = self.by_path.get(&format!("{}/{f}", slashes(d).trim_end_matches('/'))) { return Found::One(id, None); } }
        let Some(c) = self.by_name.get(&base(file)) else { return Found::Nothing };
        if c.len() == 1 { return Found::One(c[0].0, None); }
        // Same file name in several folders: the folder given (or the path's own folders) picks one.
        let tail = match folder { Some(d) => format!("/{}/{}", slashes(d).trim_matches('/'), base(file)), None => format!("/{}", f.trim_start_matches('/')) }.to_lowercase();
        let hits: Vec<i64> = c.iter().filter(|(_, p)| p.ends_with(&tail)).map(|x| x.0).collect();
        if hits.len() == 1 { Found::One(hits[0], None) } else { Found::Many }
    }

    fn by_stem(&self, stem: &str) -> Found {
        match self.by_stem.get(&stem.to_lowercase()).map(Vec::as_slice) { Some([id]) => Found::One(*id, None), Some([]) | None => Found::Nothing, _ => Found::Many }
    }

    fn by_time(&self, t: NaiveDateTime) -> Found {
        let hits: Vec<_> = self.timeline.iter().filter(|(_, a, b)| *a <= t && t < *b).collect();
        match hits.as_slice() {
            [(id, a, _)] => Found::One(*id, Some((t - *a).num_milliseconds() as f64 / 1000.0)),
            [] => Found::Nothing,
            _ => Found::Many,
        }
    }
}

/// The recording a per-file BirdNET-Analyzer table belongs to: "<recording>.BirdNET.results.csv" and the like.
fn table_stem(table: &Path) -> Option<String> {
    let name = table.file_name()?.to_string_lossy().to_string();
    let lower = name.to_lowercase();
    lower.find(".birdnet.").map(|i| name[..i].to_string())
}

fn note(list: &mut Vec<String>, v: String) { if list.len() < 10 && !list.contains(&v) { list.push(v); } }

/// Imports one results table or every table in a folder. `labels` are the installed models' labels, used to find the
/// scientific name when a table only has common names (Raven tables from BirdNET-Analyzer).
pub fn import(p: &Project, path: &Path, labels: &[Label]) -> Result<ImportReport> {
    let tables: Vec<std::path::PathBuf> = if path.is_dir() {
        walkdir::WalkDir::new(path).follow_links(true).into_iter().filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file() && !e.file_name().to_string_lossy().starts_with('.'))
            .filter(|e| e.path().extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("csv") || x.eq_ignore_ascii_case("txt")))
            .map(|e| e.into_path()).collect()
    } else { vec![path.to_path_buf()] };
    let names = Names::new(p, labels)?;
    let recs = Recordings::new(p)?;
    let mut rep = ImportReport::default();
    let mut models = BTreeSet::new();
    let mut species: HashMap<String, i64> = HashMap::new();
    let mut refit: BTreeSet<(i64, &'static str)> = BTreeSet::new();
    let tx = p.db.unchecked_transaction()?;
    for t in &tables {
        let Ok(bytes) = std::fs::read(t) else { continue };
        let Some((model, rows)) = parse(&String::from_utf8_lossy(&bytes)) else { continue };
        rep.tables += 1;
        models.insert(model);
        let stem = table_stem(t);
        for row in rows {
            rep.rows += 1;
            let found = match (&row.file, &stem) {
                (Some(f), _) => match recs.by_file(f, row.folder.as_deref()) { Found::Nothing if model == BIRDNET_PI => row.when.map_or(Found::Nothing, |w| recs.by_time(w)), x => x },
                (None, Some(s)) => recs.by_stem(s),
                (None, None) => row.when.map_or(Found::Nothing, |w| recs.by_time(w)),
            };
            let (file_id, offset) = match found {
                Found::One(id, off) => (id, off),
                Found::Many | Found::Nothing => {
                    rep.unmatched += 1;
                    let what = row.file.as_deref().map(base).or_else(|| stem.clone()).or_else(|| row.when.map(|w| w.to_string())).unwrap_or_else(|| "a row with no file or time".into());
                    note(&mut rep.unmatched_examples, what);
                    continue;
                }
            };
            rep.matched += 1;
            // BirdNET-Pi/Go rows found by time start where the time says; their 3 s clip length is BirdNET's window.
            let start = offset.or(row.start).unwrap_or(0.0).max(0.0);
            let end = if offset.is_some() { start + 3.0 } else { row.end.unwrap_or(start + 3.0) };
            let auto = names.resolve(&row.name, &row.common);
            let label = if row.name.trim().is_empty() { &row.common } else { &row.name };
            if auto.is_none() && !NOTHING.contains(&label.trim().to_lowercase().as_str()) { note(&mut rep.unknown_names, label.clone()); }
            // Kaleidoscope's MANUAL ID is a reviewer's answer: the same species confirms the automatic ID, another one replaces it.
            let manual = row.manual.as_deref().map(|m| names.resolve(m, ""));
            if let (Some(m), Some(None)) = (&row.manual, &manual) { if !NOTHING.contains(&m.to_lowercase().as_str()) { note(&mut rep.unknown_names, m.clone()); } }
            let mut put = |sp: &(String, String), verdict: Option<&str>| -> Result<()> {
                let sid = match species.get(&sp.0) { Some(&s) => s, None => { let s = p.species_id(&sp.0, &sp.1)?; species.insert(sp.0.clone(), s); s } };
                let existing: Option<i64> = tx.query_row("SELECT id FROM detections WHERE file_id = ?1 AND model = ?2 AND species = ?3 AND abs(start_s - ?4) < 0.05",
                    params![file_id, model, sid, start], |r| r.get(0)).optional()?;
                let id = match existing {
                    Some(id) => id,
                    None => {
                        rep.added += 1;
                        tx.query_row("INSERT INTO detections (file_id, model, start_s, end_s, species, confidence, in_range) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1) RETURNING id",
                            params![file_id, model, start, end, sid, row.conf], |r| r.get(0))?
                    }
                };
                if let Some(v) = verdict {
                    // Never overwrite an answer given in Studio.
                    tx.execute("INSERT INTO reviews (detection_id, verdict, at) VALUES (?1, ?2, datetime('now')) ON CONFLICT (detection_id) DO NOTHING", params![id, v])?;
                    refit.insert((sid, model));
                }
                Ok(())
            };
            match (&auto, &manual) {
                (Some(a), None) => put(a, None)?,
                (Some(a), Some(Some(m))) if a.0 == m.0 => put(a, Some("yes"))?,
                (Some(a), Some(m)) => { put(a, Some("no"))?; if let Some(m) = m { put(m, Some("yes"))?; } }
                (None, Some(Some(m))) => put(m, Some("yes"))?,
                (None, _) => rep.no_species += 1,
            }
            tx.execute("INSERT INTO file_models (file_id, model, status, error, processed_at) VALUES (?1, ?2, 'done', NULL, datetime('now'))
                ON CONFLICT (file_id, model) DO UPDATE SET status = 'done', error = NULL, processed_at = excluded.processed_at", params![file_id, model])?;
        }
    }
    tx.commit()?;
    if rep.tables == 0 { bail!("No results table Studio can read was found in {}. It reads BirdNET-Analyzer CSV and Raven tables, BirdNET-Pi and BirdNET-Go detection exports, and Kaleidoscope id.csv.", path.display()); }
    for (sid, model) in refit { crate::review::fit_threshold(p, sid, model, crate::review::TARGET)?; }
    rep.model = models.into_iter().collect::<Vec<_>>().join(", ");
    Ok(rep)
}

/// "Kaleidoscope" for "import:kaleidoscope", for summaries.
pub fn tool_name(model: &str) -> String {
    model.split(", ").map(|m| match m { ANALYZER => "BirdNET-Analyzer", BIRDNET_PI => "BirdNET-Pi or BirdNET-Go", KALEIDOSCOPE => "Kaleidoscope", x => x }).collect::<Vec<_>>().join(" and ")
}

/// One line for History.
pub fn summary(x: &ImportReport, src: &str) -> String {
    use crate::history::n;
    format!("Imported {} detections from {} ({} of {} rows matched to recordings, {} not matched) from {src}", n(x.added as i64),
        tool_name(&x.model), n(x.matched as i64), n(x.rows as i64), n(x.unmatched as i64))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::open(&dir.path().join("t.wlnw")).unwrap();
        p.db.execute_batch("INSERT INTO deployments (id, name, folder, serial, lat, lon) VALUES (1, 'A', '/d/A', '', 51.5, -0.1), (2, 'B', '/d/B', '', 51.5, -0.1);
            INSERT INTO files (id, deployment_id, path, start_local, duration_s) VALUES
              (1, 1, '/d/A/A_20250601_050000.wav', '2025-06-01 05:00:00', 600), (2, 1, '/d/A/A_20250601_051000.wav', '2025-06-01 05:10:00', NULL),
              (3, 2, '/d/B/same.wav', '2025-06-02 05:00:00', 600), (4, 1, '/d/A/same.wav', NULL, NULL);
            INSERT INTO species (scientific, common) VALUES ('Erithacus rubecula', 'European Robin');").unwrap();
        (dir, p)
    }
    fn write(dir: &Path, name: &str, body: &str) -> std::path::PathBuf { let f = dir.join(name); std::fs::write(&f, body).unwrap(); f }

    #[test]
    fn reads_tables() {
        let (h, r) = parse("Start (s),End (s),Scientific name,Common name,Confidence,File\n0.0,3.0,Turdus merula,Eurasian Blackbird,0.8123,\"/x/a, b.wav\"\n").unwrap();
        assert_eq!(h, ANALYZER);
        assert_eq!((r[0].file.as_deref(), r[0].start, r[0].end, r[0].name.as_str(), r[0].conf), (Some("/x/a, b.wav"), Some(0.0), Some(3.0), "Turdus merula", 0.8123));
        let raven = "Selection\tView\tChannel\tBegin Time (s)\tEnd Time (s)\tLow Freq (Hz)\tHigh Freq (Hz)\tCommon Name\tSpecies Code\tConfidence\tBegin Path\tFile Offset (s)\n\
            1\tSpectrogram 1\t1\t603.0\t606.0\t0\t15000\tEuropean Robin\teurrob1\t0.91\t/d/A/A_20250601_051000.wav\t3.0\n";
        let (h, r) = parse(raven).unwrap();
        assert_eq!((h, r[0].start, r[0].end, r[0].common.as_str()), (ANALYZER, Some(3.0), Some(6.0), "European Robin"));
        let (h, r) = parse("Date;Time;Sci_Name;Com_Name;Confidence;Lat;Lon;Cutoff;Week;Sens;Overlap;File_Name\n2025-06-01;05:04:30;Turdus merula;Eurasian Blackbird;0.9;51;0;0.7;22;1.25;0;Eurasian_Blackbird-90-2025-06-01-birdnet-05:04:30.mp3\n").unwrap();
        assert_eq!((h, r[0].when.unwrap().to_string().as_str()), (BIRDNET_PI, "2025-06-01 05:04:30"));
        let (h, r) = parse("INDIR,FOLDER,IN FILE,CHANNEL,OFFSET,DURATION,AUTO ID*,MATCH RATIO,MANUAL ID\n/d,B,same.wav,0,12.5,4.2,MYOLUC,0.87,\n").unwrap();
        assert_eq!((h, r[0].folder.as_deref(), r[0].start, r[0].end), (KALEIDOSCOPE, Some("B"), Some(12.5), Some(16.7)));
        assert!(parse("a,b\n1,2\n").is_none());
        assert_eq!(conf("85"), Some(0.85));
    }

    #[test]
    fn imports_and_matches() {
        let (dir, p) = project();
        let d = dir.path();
        // Combined BirdNET-Analyzer CSV: one full path, one file name only, one unknown recording.
        let f = write(d, "BirdNET_CombinedTable.csv", "Start (s),End (s),Scientific name,Common name,Confidence,File\n\
            0.0,3.0,Turdus merula,Eurasian Blackbird,0.81,/d/A/A_20250601_050000.wav\n6.0,9.0,Turdus merula,Eurasian Blackbird,0.4,A_20250601_051000.wav\n\
            0.0,3.0,Turdus merula,Eurasian Blackbird,0.9,/elsewhere/missing.wav\n");
        let r = import(&p, &f, &[]).unwrap();
        assert_eq!((r.rows, r.matched, r.unmatched, r.added, r.model.as_str()), (3, 2, 1, 2, ANALYZER));
        assert_eq!(r.unmatched_examples, vec!["missing.wav".to_string()]);
        // Importing again adds nothing.
        assert_eq!(import(&p, &f, &[]).unwrap().added, 0);
        // file_models marks the recordings, so the Files screen lists the import.
        let n: i64 = p.db.query_row("SELECT COUNT(*) FROM file_models WHERE model = ?1 AND status = 'done'", [ANALYZER], |r| r.get(0)).unwrap();
        assert_eq!(n, 2);

        // A per-file Raven table: common name only, resolved through the species table; the file comes from the table's name.
        let sub = d.join("raven"); std::fs::create_dir_all(&sub).unwrap();
        write(&sub, "A_20250601_051000.BirdNET.selection.table.txt", "Selection\tView\tChannel\tBegin Time (s)\tEnd Time (s)\tLow Freq (Hz)\tHigh Freq (Hz)\tCommon Name\tSpecies Code\tConfidence\n1\tSpectrogram 1\t1\t3.0\t6.0\t0\t15000\tEuropean Robin\teurrob1\t0.91\n\
            2\tSpectrogram 1\t1\t9.0\t12.0\t0\t15000\tNo Such Bird\tnosuch\t0.5\n");
        write(&sub, "notes.txt", "nothing to see\n");
        let r = import(&p, &sub, &[]).unwrap();
        assert_eq!((r.tables, r.matched, r.added, r.no_species), (1, 2, 1, 1));
        assert_eq!(r.unknown_names, vec!["No Such Bird".to_string()]);

        // BirdNET-Pi: matched by time. 05:04:30 is in file 1 at 270 s; 05:12:00 falls in file 2, whose length is not known yet.
        let f = write(d, "detections.csv", "Date,Time,Sci_Name,Com_Name,Confidence,File_Name\n2025-06-01,05:04:30,Erithacus rubecula,European Robin,0.88,clip1.mp3\n\
            2025-06-01,05:12:00,Erithacus rubecula,European Robin,0.7,clip2.mp3\n2025-07-01,05:00:00,Erithacus rubecula,European Robin,0.7,clip3.mp3\n");
        let r = import(&p, &f, &[]).unwrap();
        assert_eq!((r.matched, r.unmatched), (2, 1));
        let s: f64 = p.db.query_row("SELECT start_s FROM detections WHERE model = ?1 AND file_id = 1", [BIRDNET_PI], |r| r.get(0)).unwrap();
        assert!((s - 270.0).abs() < 1e-6);

        // Kaleidoscope: "same.wav" is in two folders; FOLDER picks B. MANUAL ID confirms one, corrects another, rejects noise.
        let f = write(d, "id.csv", "INDIR,FOLDER,IN FILE,CHANNEL,OFFSET,DURATION,AUTO ID*,MATCH RATIO,MANUAL ID\n\
            /d,B,same.wav,0,1.0,4.0,MYOLUC,0.87,MYOLUC\n/d,B,same.wav,0,8.0,4.0,MYOLUC,0.6,EPTFUS\n/d,B,same.wav,0,20.0,4.0,LASBOR,0.5,Noise\n/d,B,same.wav,0,30.0,4.0,Noise,,\n");
        let r = import(&p, &f, &[]).unwrap();
        assert_eq!((r.matched, r.unmatched, r.added, r.no_species), (4, 0, 4, 1));
        let v: Vec<(String, String)> = p.db.prepare("SELECT s.scientific, r.verdict FROM detections d JOIN species s ON s.id = d.species JOIN reviews r ON r.detection_id = d.id WHERE d.model = ?1 AND d.file_id = 3 ORDER BY d.start_s, s.scientific").unwrap()
            .query_map([KALEIDOSCOPE], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|x| x.unwrap()).collect();
        assert_eq!(v, vec![("Myotis lucifugus".into(), "yes".into()), ("Eptesicus fuscus".into(), "yes".into()), ("Myotis lucifugus".into(), "no".into()), ("Lasiurus borealis".into(), "no".into())]);

        // Imported detections show in review per model and in exports.
        let sp = crate::review::species(&p).unwrap();
        assert!(sp.iter().any(|s| s.model == KALEIDOSCOPE && s.scientific == "Myotis lucifugus" && s.reviewed == 2));
        let robin: i64 = p.db.query_row("SELECT id FROM species WHERE scientific = 'Erithacus rubecula'", [], |r| r.get(0)).unwrap();
        assert!(!crate::review::queue(&p, robin, BIRDNET_PI, 10).unwrap().is_empty());
        let acc = p.accepted().unwrap();
        assert!(acc.iter().any(|d| d.model == KALEIDOSCOPE && d.scientific == "Eptesicus fuscus"));
        assert!(!acc.iter().any(|d| d.scientific == "Lasiurus borealis"));
        assert!(import(&p, &d.join("raven").join("notes.txt"), &[]).is_err());
        // Kaleidoscope export: yes verdicts fill MANUAL ID; the table reads back as Kaleidoscope.
        let out = d.join("out_id.csv");
        assert_eq!(crate::export::kaleidoscope(&p, &out).unwrap(), acc.len());
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.starts_with("FOLDER,IN FILE,OFFSET,DURATION,AUTO ID*,MATCH RATIO,MANUAL ID\n"), "{text}");
        assert!(text.contains("/d/B,same.wav,8.00,4.00,Eptesicus fuscus,0.6000,Eptesicus fuscus"), "{text}");
        assert_eq!(parse(&text).unwrap().0, KALEIDOSCOPE);
        assert_eq!(crate::export::model_name(KALEIDOSCOPE), "Kaleidoscope");
    }
}
