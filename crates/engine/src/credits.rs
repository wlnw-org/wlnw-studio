//! Who made each model Studio runs, under which licence, and how to cite it. Shown on the Credits and licences screen and
//! written into every export that contains the model's results (CSV column, Darwin Core identificationReferences and
//! eml.xml, WDX classifier reference, Camtrap DP sources, and a CITATION.txt next to Raven, Audacity and Kaleidoscope files).
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Credit {
    /// Studio's model id, or a short key for parts that are not a selectable model.
    pub id: &'static str,
    pub name: &'static str,
    pub by: &'static str,
    pub licence: &'static str,
    pub licence_url: &'static str,
    /// The citation the authors ask for.
    pub citation: &'static str,
    /// Link to the paper (DOI or arXiv).
    pub paper_url: &'static str,
    /// The authors' own page or repository.
    pub source_url: &'static str,
    /// One plain sentence on how Studio gets or uses it.
    pub note: &'static str,
}

pub const CREDITS: &[Credit] = &[
    Credit {
        id: "birdnet-2.4", name: "BirdNET 2.4",
        by: "K. Lisa Yang Center for Conservation Bioacoustics at the Cornell Lab of Ornithology, and Chemnitz University of Technology",
        licence: "CC BY-NC-SA 4.0", licence_url: "https://creativecommons.org/licenses/by-nc-sa/4.0/",
        citation: "Kahl, S., Wood, C. M., Eibl, M. and Klinck, H. (2021). BirdNET: A deep learning solution for avian diversity monitoring. Ecological Informatics 61, 101236. https://doi.org/10.1016/j.ecoinf.2021.101236",
        paper_url: "https://doi.org/10.1016/j.ecoinf.2021.101236", source_url: "https://github.com/birdnet-team/BirdNET-Analyzer",
        note: "Used with the permission of the BirdNET team for research and conservation. The model keeps its own non-commercial licence.",
    },
    Credit {
        id: "perch-2.0", name: "Perch 2.0", by: "Google DeepMind and Google Research",
        licence: "Apache 2.0", licence_url: "https://www.apache.org/licenses/LICENSE-2.0",
        citation: "van Merriënboer, B. et al. (2025). Perch 2.0: The Bittern Lesson for Bioacoustics. arXiv:2508.04665. https://arxiv.org/abs/2508.04665",
        paper_url: "https://arxiv.org/abs/2508.04665", source_url: "https://www.kaggle.com/models/google/bird-vocalization-classifier",
        note: "Downloaded on first use from a checked ONNX copy.",
    },
    Credit {
        id: "batdetect2-uk", name: "BatDetect2 (UK model)", by: "Oisin Mac Aodha, Santiago Martínez Balvanera and contributors",
        licence: "CC BY-NC 4.0", licence_url: wlnw_batdetect2::LICENCE_URL,
        citation: "Mac Aodha, O. et al. (2022). Towards a General Approach for Bat Echolocation Detection and Classification. bioRxiv. https://doi.org/10.1101/2022.12.14.520490",
        paper_url: wlnw_batdetect2::PAPER_URL, source_url: wlnw_batdetect2::REPO_URL,
        note: "Non-commercial use only. Studio's BatDetect2 code is also CC BY-NC 4.0. You make the model files from the official BatDetect2 package.",
    },
    Credit {
        id: "speciesnet", name: "SpeciesNet 4.0.3a", by: "Google",
        licence: "Apache 2.0", licence_url: "https://www.apache.org/licenses/LICENSE-2.0",
        citation: "Gadot, T. et al. (2024). To crop or not to crop: Comparing whole-image and cropped classification on a large dataset of camera trap images. IET Computer Vision. https://doi.org/10.1049/cvi2.12318",
        paper_url: "https://doi.org/10.1049/cvi2.12318", source_url: "https://github.com/google/cameratrapai",
        note: "Names the animals in camera trap photos.",
    },
    Credit {
        id: "megadetector", name: "MegaDetector v6 MIT", by: "Microsoft AI for Good Lab (Pytorch-Wildlife)",
        licence: "MIT code, CC BY 4.0 weights", licence_url: "https://creativecommons.org/licenses/by/4.0/",
        citation: "Hernandez, A. et al. (2024). Pytorch-Wildlife: A Collaborative Deep Learning Framework for Conservation. arXiv:2405.12930. https://arxiv.org/abs/2405.12930",
        paper_url: "https://arxiv.org/abs/2405.12930", source_url: "https://github.com/microsoft/CameraTraps",
        note: "Finds animals, people and vehicles in camera trap photos.",
    },
];

/// The credit for a detection's model id. Imported BirdNET results credit BirdNET; Studio's own classifier has none.
pub fn for_model(model: &str) -> Option<&'static Credit> {
    let id = match model {
        m if m.starts_with("birdnet") || m == "import:birdnet-analyzer" || m == "import:birdnet-pi" => "birdnet-2.4",
        m if m.starts_with("perch") => "perch-2.0",
        m if m.starts_with("batdetect2") => "batdetect2-uk",
        m if m.starts_with("speciesnet") => "speciesnet",
        m if m.starts_with("megadetector") => "megadetector",
        _ => return None,
    };
    CREDITS.iter().find(|c| c.id == id)
}

/// The citation to write next to a detection, or "" when the model needs none.
pub fn reference(model: &str) -> &'static str { for_model(model).map_or("", |c| c.citation) }

/// The text of a CITATION.txt for the models in an export (each listed once, in CREDITS order).
pub fn citation_file(models: &[&str]) -> String {
    let used: Vec<&Credit> = CREDITS.iter().filter(|c| models.iter().any(|m| for_model(m).is_some_and(|x| x.id == c.id))).collect();
    let mut s = format!("These results were made with {}.\n", crate::export::STUDIO);
    if used.is_empty() { return s; }
    s.push_str("\nPlease cite the models that identified the species:\n");
    for c in used {
        s.push_str(&format!("\n{}\nBy {}. Licence {} ({}).\n{}\nSource {}\n", c.name, c.by, c.licence, c.licence_url, c.citation, c.source_url));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credits_cover_every_model() {
        for m in ["birdnet-2.4", "import:birdnet-pi", "perch-2.0", "batdetect2-uk", "speciesnet-4.0.3a", "megadetector-v6"] {
            assert!(!reference(m).is_empty(), "{m}");
        }
        assert_eq!(reference("wnc-1"), "");
        let t = citation_file(&["batdetect2-uk", "batdetect2-uk"]);
        assert_eq!(t.matches("BatDetect2 (UK model)").count(), 1);
        assert!(t.contains("https://doi.org/10.1101/2022.12.14.520490") && t.contains("https://github.com/macaodha/batdetect2"));
        for c in CREDITS { assert!(!c.citation.contains('\u{2014}') && !c.note.contains('\u{2014}')); }
    }
}
