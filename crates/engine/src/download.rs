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
        // Studio's ONNX conversion of BirdNET 2.4, packaged with the BirdNET team's permission (Stefan Kahl, email of 11 Nov 2026,
        // docs/PERMISSIONS.md), for the wlnw-studio "models-birdnet-2.4" pre-release with its licence and notice
        // (docs/model-packs/birdnet-2.4). `downloadable` stays false until that release is published: the files and checksums
        // below are the ones already used by every Studio that has BirdNET, so adding them by hand keeps working the same.
        files: &[
            PackFile { name: "birdnet_v24_cnn.onnx", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-birdnet-2.4/birdnet_v24_cnn.onnx"], sha256: "66aec7585e42831bf628968380c8f285936610c0910b87890cadb411bdfa7f4c", size: 51_101_259 },
            PackFile { name: "birdnet_v24_meta.onnx", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-birdnet-2.4/birdnet_v24_meta.onnx"], sha256: "022831060cd6904b42d0e826cc17b5cc6d9697fb521e8d10446d9076d7a78b0a", size: 29_525_525 },
            PackFile { name: "w1.f32", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-birdnet-2.4/w1.f32"], sha256: "e689f7ccf093a825f8c6252e583e432911bd546aa2ae18c9381b891addb455ec", size: 8_192 },
            PackFile { name: "w2.f32", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-birdnet-2.4/w2.f32"], sha256: "4a80fad778afac0a915dbe252979e9bc65fc8b89925e6b77cded5be3df173903", size: 4_096 },
            PackFile { name: "M1.f32", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-birdnet-2.4/M1.f32"], sha256: "f9ed152d0484df13c15a28715c6b6a960df97fa48ba0f68eb0f536160bad9e55", size: 393_600 },
            PackFile { name: "M2.f32", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-birdnet-2.4/M2.f32"], sha256: "927b924d2d8f4d0d42c461813b8b63b752ec04776b10506a593e0754db279a51", size: 196_992 },
            PackFile { name: "mel.json", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-birdnet-2.4/mel.json"], sha256: "6e9badb1415c161cb0a5c0670d126d55d22eaaf78bfa87540f0c28e861c713e5", size: 116 },
            PackFile { name: "labels.txt", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-birdnet-2.4/labels.txt"], sha256: "7399780d6743a806d79857f3dea16d8be2ecbd44b745f7e378555bdb7742e685", size: 266_261 },
            PackFile { name: "BirdNET-LICENSE.txt", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-birdnet-2.4/BirdNET-LICENSE.txt"], sha256: "e66c269d4819aaab34b49ef5220c4ddab6756f21bb5180761a4eb8561f2b7bbd", size: 20_850 },
            PackFile { name: "BirdNET-NOTICE.txt", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-birdnet-2.4/BirdNET-NOTICE.txt"], sha256: "12ae7acae43630802d6c6b1092e19e830d5f95c90f44bcb138e1c0af05b4b629", size: 998 },
        ],
        downloadable: false,
        note: "One-click download coming soon. If you have the Studio BirdNET files, add them from a folder.",
    },
    Pack {
        id: "batdetect2-uk", name: "BatDetect2 (UK bats)",
        description: "Mac Aodha et al.'s bat model: finds each echolocation call in ultrasonic recordings and names 17 UK bat species. The default for recordings at 192 kHz and above.",
        licence: "CC-BY-NC-4.0", licence_url: "https://creativecommons.org/licenses/by-nc/4.0/", dir: "batdetect2",
        files: &[],
        downloadable: false,
        note: "BatDetect2's authors publish the model only in their own package, so Wildlife Network does not host a copy. Make the three files from the official package with crates/batdetect2/python/export_model.py (see the guide on wlnw.org/help), then add them from a folder. Non-commercial use only.",
    },
    Pack {
        id: "speciesnet-4.0.3a", name: "SpeciesNet (camera traps)",
        description: "Google's camera-trap model with Microsoft's MegaDetector v6: finds animals, people and vehicles in photos and names about 2,000 species and groups, checked against the camera's country.",
        licence: "Apache-2.0", licence_url: "https://www.apache.org/licenses/LICENSE-2.0", dir: "speciesnet",
        // Converted files hosted on the wlnw-studio "models-speciesnet-4.0.3a" pre-release (never "latest", so the updater ignores it).
        // SpeciesNet Apache 2.0; MegaDetector v6 MIT weights CC BY 4.0: credits in NOTICE.txt.
        files: &[
            PackFile { name: "detector.onnx", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-speciesnet-4.0.3a/detector.onnx"], sha256: "742e467c4e56b699d69f805545dfcbbc7252a67fe39b0f1ba0e6590a68c4a86b", size: 29_141_004 },
            PackFile { name: "speciesnet_v403a.onnx", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-speciesnet-4.0.3a/speciesnet_v403a.onnx"], sha256: "4621fc1472b3889a3dcce755a9c9bb026f29665884228959b250fc40ddf69d17", size: 223_998_767 },
            PackFile { name: "labels.txt", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-speciesnet-4.0.3a/labels.txt"], sha256: "e0c41415d9b29dfd6d2e2494205d1197d28595cc6f69cb129495f9827155547e", size: 255_693 },
            PackFile { name: "taxonomy.txt", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-speciesnet-4.0.3a/taxonomy.txt"], sha256: "4bd5d1a85a63857f557d487c5d86c0911173c59b34977ebc0ee909484ce277f2", size: 357_417 },
            PackFile { name: "geofence.json", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-speciesnet-4.0.3a/geofence.json"], sha256: "27a88e932e411f73a4937c935fe36cf61a9060a1c488f4c69ade3f19eb8507d1", size: 6_264_811 },
            PackFile { name: "speciesnet.json", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-speciesnet-4.0.3a/speciesnet.json"], sha256: "5e8dfa016f6b63ac688df9b48e19fdef0f9a66a8caa1fb242a3090745a000cc7", size: 206 },
            PackFile { name: "NOTICE.txt", urls: &["https://github.com/wlnw-org/wlnw-studio/releases/download/models-speciesnet-4.0.3a/NOTICE.txt"], sha256: "51e3321b81553dc3b218bdfb45a523d7425701e0d1413ec92b880d9fc6406d0f", size: 1_440 },
        ],
        downloadable: true,
        note: "",
    },
];

/// Files that must be present for a pack to count as installed (BatDetect2 has no download list, and BirdNET's licence and notice
/// files are not required, so a BirdNET added by hand still counts).
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
