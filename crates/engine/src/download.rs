//! First-run model download. Each pack is a list of files with a fixed source (a pinned version), a fallback source
//! and a SHA-256; a file is only used after its checksum matches. Interrupted downloads resume where they stopped.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Serialize)]
pub struct PackFile { pub name: &'static str, pub urls: &'static [&'static str], pub sha256: &'static str, pub size: u64 }

#[derive(Debug, Clone, Serialize)]
pub struct Pack {
    pub id: &'static str, pub name: &'static str, pub description: &'static str, pub licence: &'static str, pub licence_url: &'static str,
    /// Folder inside the models folder ("" for the models folder itself).
    pub dir: &'static str,
    pub files: &'static [PackFile],
    /// False while the files may not be redistributed (the app then explains how to add them by hand).
    pub downloadable: bool,
    pub note: &'static str,
}

const PERCH_MODEL: &[&str] = &[
    "https://huggingface.co/Calotriton/perch-v2-onnx/resolve/bb0f724ee99fa4e0dc3637d8bbdc6a0e26660480/perch_v2.onnx",
    "https://huggingface.co/justinchuby/Perch-onnx/resolve/da88bf8c37c0b3068fc706fe509f633b9fbd28c1/perch_v2.onnx",
];

pub const CATALOG: &[Pack] = &[
    Pack {
        id: "perch-2.0", name: "Perch 2.0",
        description: "Google's bioacoustics model: about 15,000 species of birds, frogs, insects and mammals. The default in Africa and South Asia, and the base of the Wildlife Network Classifier.",
        licence: "Apache-2.0", licence_url: "https://www.apache.org/licenses/LICENSE-2.0", dir: "perch_v2",
        files: &[
            PackFile { name: "perch_v2.onnx", urls: PERCH_MODEL, sha256: "bf0c8467a924cb074663970ca4a0ab1e143602121930209657d0dff5d5cefa1f", size: 409_148_616 },
            PackFile { name: "labels.csv", urls: &["https://huggingface.co/Calotriton/perch-v2-onnx/resolve/bb0f724ee99fa4e0dc3637d8bbdc6a0e26660480/labels.csv"],
                sha256: "e4d5c0397d8fb08bf90c6b13a34810af53504faad927e472fcc567793c9de057", size: 312_716 },
            PackFile { name: "LICENSE", urls: &["https://huggingface.co/Calotriton/perch-v2-onnx/resolve/bb0f724ee99fa4e0dc3637d8bbdc6a0e26660480/LICENSE"],
                sha256: "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30", size: 11_358 },
        ],
        downloadable: true, note: "",
    },
    Pack {
        id: "birdnet-2.4", name: "BirdNET 2.4",
        description: "The Cornell Lab and Chemnitz University's bird model: about 6,500 species, filtered by place and week. Fast.",
        licence: "CC-BY-NC-SA-4.0", licence_url: "https://creativecommons.org/licenses/by-nc-sa/4.0/", dir: "",
        files: &[],
        downloadable: false,
        note: "Not downloadable yet: we have asked the BirdNET team for permission to distribute it. If you have the Studio BirdNET files, add them from a folder.",
    },
    Pack {
        id: "batdetect2-uk", name: "BatDetect2 (UK bats)",
        description: "Mac Aodha et al.'s bat model: finds each echolocation call in ultrasonic recordings and names 17 UK bat species. The default for recordings at 192 kHz and above.",
        licence: "CC-BY-NC-4.0", licence_url: "https://creativecommons.org/licenses/by-nc/4.0/", dir: "batdetect2",
        files: &[],
        downloadable: false,
        note: "Not downloadable yet: BatDetect2 is for non-commercial use only and no converted copy is published. Make the three files with spike/batdetect2_export.py (see docs/SPIKE.md), then add them from a folder.",
    },
    Pack {
        id: "speciesnet-4.0.3a", name: "SpeciesNet (camera traps)",
        description: "Google's camera-trap model with Microsoft's MegaDetector v6: finds animals, people and vehicles in photos and names about 2,000 species and groups, checked against the camera's country.",
        licence: "Apache-2.0", licence_url: "https://www.apache.org/licenses/LICENSE-2.0", dir: "speciesnet",
        files: &[],
        downloadable: false,
        note: "Not downloadable yet: the converted files are not published anywhere. Both models may be shared (SpeciesNet Apache 2.0, MegaDetector v6 MIT weights CC BY 4.0). Make the six files with spike/speciesnet_export.py (see docs/SPIKE.md), then add them from a folder.",
    },
];

/// Files that must be present for a pack to count as installed (BirdNET and BatDetect2 have no download list yet, so their files are named here).
pub fn required(pack: &Pack) -> Vec<&'static str> {
    match pack.id {
        "birdnet-2.4" => vec!["birdnet_v24_cnn.onnx", "birdnet_v24_meta.onnx", "w1.f32", "w2.f32", "M1.f32", "M2.f32", "mel.json", "labels.txt"],
        "batdetect2-uk" => vec!["batdetect2_uk.onnx", "batdetect2.json", "labels.txt"],
        "speciesnet-4.0.3a" => crate::camtrap::FILES.to_vec(),
        _ => pack.files.iter().map(|f| f.name).collect(),
    }
}

pub fn installed(pack: &Pack, models_dir: &Path) -> bool {
    let d = models_dir.join(pack.dir);
    required(pack).iter().all(|f| d.join(f).exists())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop { let n = f.read(&mut buf)?; if n == 0 { break; } h.update(&buf[..n]); }
    Ok(hex::encode(h.finalize()))
}

/// Downloads every file of a pack into `models_dir/<pack.dir>`. `progress(done, total)` is in bytes over the whole pack.
pub fn install(pack: &Pack, models_dir: &Path, stop: &AtomicBool, mut progress: impl FnMut(u64, u64)) -> Result<()> {
    if !pack.downloadable { bail!("{}", pack.note); }
    let dir = models_dir.join(pack.dir);
    std::fs::create_dir_all(&dir)?;
    let total: u64 = pack.files.iter().map(|f| f.size).sum();
    let mut before = 0u64;
    for file in pack.files {
        let dest = dir.join(file.name);
        if dest.exists() && sha256_file(&dest)? == file.sha256 { before += file.size; progress(before, total); continue; }
        let part = dir.join(format!("{}.part", file.name));
        let mut last_err = None;
        for url in file.urls {
            match fetch(url, &part, file.size, stop, |n| progress(before + n, total)) {
                Ok(()) => { last_err = None; break; }
                Err(e) => {
                    if stop.load(Ordering::Relaxed) { bail!("Download stopped. It will continue where it left off."); }
                    last_err = Some(e);
                }
            }
        }
        if let Some(e) = last_err { return Err(e).with_context(|| format!("could not download {}", file.name)); }
        let got = sha256_file(&part)?;
        if got != file.sha256 {
            std::fs::remove_file(&part).ok();
            bail!("{} did not match its checksum, so it was not used. Please try again.", file.name);
        }
        std::fs::rename(&part, &dest)?;
        before += file.size;
        progress(before, total);
    }
    Ok(())
}

fn fetch(url: &str, part: &Path, size: u64, stop: &AtomicBool, mut progress: impl FnMut(u64)) -> Result<()> {
    let have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    if have > size { std::fs::remove_file(part)?; }
    let have = have.min(size);
    if have == size { return Ok(()); }
    let mut req = ureq::get(url).set("user-agent", concat!("WildlifeNetworkStudio/", env!("CARGO_PKG_VERSION")));
    if have > 0 { req = req.set("range", &format!("bytes={have}-")); }
    let res = req.call().map_err(|e| anyhow::anyhow!("{url}: {e}"))?;
    // A server that ignores the range starts again from the beginning.
    let resumed = res.status() == 206;
    let mut out = std::fs::OpenOptions::new().create(true).write(true).append(resumed).truncate(!resumed).open(part)?;
    let mut done = if resumed { have } else { 0 };
    let mut r = res.into_reader();
    let mut buf = vec![0u8; 1 << 20];
    let mut since = 0u64;
    loop {
        if stop.load(Ordering::Relaxed) { bail!("stopped"); }
        let n = r.read(&mut buf)?;
        if n == 0 { break; }
        out.write_all(&buf[..n])?;
        done += n as u64;
        since += n as u64;
        if since >= 4 << 20 { progress(done); since = 0; }
    }
    out.flush()?;
    progress(done);
    if done < size { bail!("{url}: the download ended early ({done} of {size} bytes)"); }
    Ok(())
}

/// Copies model files the user already has (for example BirdNET's) from a folder into the models folder.
/// Returns the packs that are complete afterwards.
pub fn add_from_folder(src: &Path, models_dir: &Path) -> Result<Vec<&'static str>> {
    let mut copied = 0;
    for pack in CATALOG {
        let dest = models_dir.join(pack.dir);
        let names = required(pack);
        // Files straight in `src` belong to this pack only when its first file (the model) is there too: several packs
        // have a labels.txt.
        let direct = names.first().is_some_and(|f| src.join(f).exists());
        for name in names {
            let cands = if direct { vec![src.join(name), src.join(pack.dir).join(name)] } else if pack.dir.is_empty() { vec![] } else { vec![src.join(pack.dir).join(name)] };
            for cand in cands {
                if cand.exists() && !dest.join(name).exists() {
                    std::fs::create_dir_all(&dest)?;
                    std::fs::copy(&cand, dest.join(name))?;
                    copied += 1;
                    break;
                }
            }
        }
    }
    if copied == 0 { bail!("No Studio model files were found in that folder."); }
    Ok(CATALOG.iter().filter(|p| installed(p, models_dir)).map(|p| p.id).collect())
}
