# BioCLIP 2 as a second opinion next to SpeciesNet: evaluation

Run 2026-11-27. Test only, no change to Studio.

## 1. Licences

| What | Licence | Source checked | Stops Studio shipping it to free users? |
|---|---|---|---|
| BioCLIP 2 weights (imageomics/bioclip-2, Hugging Face, revision 2957b32, DOI 10.57967/hf/5765) | MIT | Hugging Face model card metadata (`license: mit`) | No. MIT allows download, use and redistribution; keep the copyright notice and cite Gu et al. 2025. |
| SPARROW Model Zoo 0.33.0, Zenodo record 23992557 (latest version on 2026-11-27; concept record 20348978) | Record: "other-open", an umbrella note: each model keeps its own upstream terms. Package `bioclip-2` and `bioclip-2-fp16`: MIT, hosting "Hosted", commercial use "allowed", restriction "attribution", conversion permission "verified" | Zenodo API, bundle LICENSE.md, MODEL_ZOO_CATALOGUE.md, the package's LICENSE.md and SOURCE.md | No. Redistributing the zoo bundle needs the per-model LICENSE.md, MODEL_CARD.md, ATTRIBUTION.md and SOURCE.md kept with it; Studio would ship those files with the download. |
| BioCLIP 2 text tower (used offline only, to make the species embedding table) | MIT (same repository) | as above | No. Only its output (one vector per species) would ship. |
| TreeOfLife-200M (training data; metadata only, not used here) | CC0 for the compilation; images keep their own licences | dataset card | Not shipped. Only used to choose a test set it cannot contain. |
| iNaturalist photos (test set a) | Per photo: only CC0, CC BY 4.0 and CC BY-NC 4.0 taken; each photo's licence and observer are in testset_inat.csv | iNaturalist API v1 `license_code` per photo | No. Used locally for this measurement only, never redistributed or committed. |
| LILA BC camera-trap images (test set b) | Community Data License Agreement, permissive 1.0 (CDLA-Permissive-1.0) | IDLE-OO Camera Traps dataset card and LILA dataset pages | No. Used locally, not committed. |
| Our footage (test set c): a wetland video shared privately with us for the Studio demo (25 Nov 2026) and the Camera survey demo images | Not published; the provenance of the 13 Camera survey images is not recorded anywhere I could find (EXIF Make "Camera trap", dates 2016) | local files | No. Private, used locally only; paths in the CSV are relative and no image is committed. Unverified: the Camera survey images' original source and licence. |

None of these stops Studio from downloading and running BioCLIP 2 for free users. The obligations are attribution (MIT notice, the BioCLIP 2 citation, the zoo's per-model files) and, for SpeciesNet, which Studio already ships, Apache 2.0.

### What BioCLIP 2 was trained on

TreeOfLife-200M combines GBIF (occurrence snapshot of 2024-05-01, DOI 10.15468/dl.bfv433, still images only; this includes iNaturalist research-grade photos and about 596,000 camera-trap images published to GBIF), Encyclopedia of Life (media manifest, no version date given), BIOSCAN-5M and FathomNet. LILA BC camera-trap sets are not among the sources, and five of them (IDLE-OO Camera Traps) were held out as BioCLIP 2's own benchmark, with MD5 and metadata checks against training. So:

- iNaturalist photos are only safe if the observation was made and uploaded after the snapshot. EOL also republishes iNaturalist photos and gives no snapshot date, so this test takes observations made and uploaded on or after 2025-06-01, after the BioCLIP 2 paper (May 2025), as a margin.
- LILA BC images are outside the training sources. They are not outside SpeciesNet's: SpeciesNet was trained on many LILA sets, so on test set b SpeciesNet is likely to have seen the images and has an advantage.

# Earlier feasibility check (2026-11-26)

Run 2026-11-26 with sparrow-engine 0.1.32, the zoo's bioclip-2 encoder (Sparrow Model Zoo 0.33.0, Zenodo record 23992557) and BioCLIP 2's text tower from imageomics/bioclip-2 (open_clip).

## Result

| On 20 video sightings | Top 1 | Top 5 |
|---|---|---|
| BioCLIP 2 zero-shot, MegaDetector crop | 18 of 20 (90%) | 20 of 20 (100%) |
| BioCLIP 2 zero-shot, whole frame | 16 of 20 (80%) | |
| SpeciesNet 4.0.3a (Studio today, species level) | 15 of 20 (75%) | |

True species among GBIF's nearby list: 20 of 20 (100%). SPARROW's encoder and open_clip agree on the same picture (cosine 0.9997).

| Site | Species offered (GBIF, within 1 degree) | Seconds per crop (CPU) |
|---|---|---|
| Bulgaria | 246 | 2.19 |
| Santa Fé do Sul and Rubinéia, São Paulo, Brazil | 234 | 1.52 |
| Middle Franconia, Germany | 471 | 2.34 |
| Organ Mountains, New Mexico, USA | 381 | 1.74 |
| southern Oregon, USA | 273 | 2.82 |

## Each sighting

| Video | Truth | SpeciesNet | BioCLIP crop, top 1 | BioCLIP whole frame |
|---|---|---|---|---|
| Badger_cubs.mp4 0 to 30 s | Meles meles | Meles meles (yes) | Meles meles 0.92 (yes) | Meles meles (yes) |
| Badger_in_the_snow.mp4 0 to 5 s | Meles meles | Meles meles (yes) | Meles meles 0.94 (yes) | Meles meles (yes) |
| Mutual_grooming_of_badgers_in_Bulgaria.mp4 0 to 5 s | Meles meles | Meles meles (yes) | Meles meles 0.98 (yes) | Meles meles (yes) |
| Cubs_Puma_concolor_capricornensis_Atlantic_Forest_Brazil.mp4 0 to 3 s | Puma concolor | Puma concolor (yes) | Puma concolor 0.99 (yes) | Puma concolor (yes) |
| Female-Puma-foraging-Atlantic-Forest-Sao-Paulo-Brazil.mp4 0 to 6 s | Puma concolor | Puma concolor (yes) | Puma concolor 1.00 (yes) | Puma concolor (yes) |
| Giant_anteater_in_Santa_Fe_do_Sul_Hunter_camera.mp4 0 to 9 s | Myrmecophaga tridactyla | Myrmecophaga tridactyla (yes) | Myrmecophaga tridactyla 0.99 (yes) | Myrmecophaga tridactyla (yes) |
| Jaguarundi-santa-fe-do-sul-brazil-camera-trap.mp4 0 to 10 s | Herpailurus yagouaroundi | (none) (no) | Puma yagouaroundi 1.00 (yes) | Puma yagouaroundi (yes) |
| Lowland-tapir-foraging-Atlantic-Forest-Sao-Paulo-Brazil-trail-camera.mp4 0 to 10 s | Tapirus terrestris | Tapirus terrestris (yes) | Tapirus terrestris 0.64 (yes) | Tapirus terrestris (yes) |
| Male-Puma-foraging-Atlantic-Forest-Sao-Paulo-Brazil.mp4 0 to 6 s | Puma concolor | Puma concolor (yes) | Mazama americana 0.65 (no) | Mazama americana (no) |
| Ocelot-foraging-Rubineia-Brazil.mp4 0 to 11 s | Leopardus pardalis | Leopardus pardalis (yes) | Leopardus pardalis 0.83 (yes) | Leopardus guttulus (no) |
| Ocelot-foraging-Sao-Paulo-Brazil.mp4 0 to 15 s | Leopardus pardalis | Leopardus pardalis (yes) | Leopardus pardalis 0.53 (yes) | Sylvilagus brasiliensis (no) |
| Tamandua_tetradactyla_Rubineia_Hunter_Camera.mp4 0 to 5 s | Tamandua tetradactyla | Tamandua tetradactyla (yes) | Tamandua tetradactyla 0.96 (yes) | Tamandua tetradactyla (yes) |
| Martes_foina-(drkb)-1.mp4 0 to 17 s | Martes foina | Animalia (no) | Martes martes 0.27 (no) | Martes foina (yes) |
| Gray_fox_(Urocyon_cinereoargenteus)_in_New_Mexico.mp4 0 to 31 s | Urocyon cinereoargenteus | Urocyon cinereoargenteus (yes) | Urocyon cinereoargenteus 0.89 (yes) | Urocyon cinereoargenteus (yes) |
| Ringtail_(Bassariscus_astutus).mp4 0 to 24 s | Bassariscus astutus | Carnivora (no) | Bassariscus astutus 0.52 (yes) | Bassariscus astutus (yes) |
| Black_bear_-_trail_camera.mp4 0 to 29 s | Ursus americanus | Ursus americanus (yes) | Ursus americanus 0.99 (yes) | Ursus americanus (yes) |
| Cougar_-_trail_camera.mp4 0 to 36 s | Puma concolor | Puma concolor (yes) | Puma concolor 1.00 (yes) | Puma concolor (yes) |
| Fishers_(Martes_pennanti)_in_Oregon.mp4 0 to 3 s | Pekania pennanti | (none) (no) | Pekania pennanti 0.52 (yes) | Martes caurina (no) |
| Badger_cubs.mp4 0 to 7 s | Meles meles | Mustelidae (no) | Meles meles 0.39 (yes) | Meles meles (yes) |
| Badger_cubs.mp4 2 to 21 s | Meles meles | Meles meles (yes) | Meles meles 0.99 (yes) | Meles meles (yes) |

## What this does and does not show

- It works end to end: SPARROW-Engine 0.1.32 loads a classifier made from the zoo's bioclip-2 encoder with a zero-shot head and a plain manifest.toml (one change needed: SPARROW refuses geo_scope "regional" without a region list, so the manifest says "foundational").
- On these 20 sightings BioCLIP on the MegaDetector crop was right 18 times (SpeciesNet 15) and always had the right species in its top 5. It named the jaguarundi, fisher and ringtail that SpeciesNet missed or left at family level. Its two misses: a male puma called red brocket deer (score 0.65) and a beech marten called pine marten (0.27, low). Crops beat whole frames.
- 20 sightings from 18 public videos is far too few to trust the numbers: an error rate between roughly 1% and 30% fits 18 of 20. The videos are on Wikimedia Commons, and BioCLIP 2's training data (TreeOfLife-200M) draws on public image collections, so some frames may be close to training images. SpeciesNet's species-level answers are also held back by its own rule of falling back to a family when unsure, which this table counts as wrong.
- The GBIF limit did its job: the true species was in every site's list (234 to 471 mammals and common birds within 1 degree). Without it BioCLIP would choose among about 450,000 taxa.
- Cost: about 1.5 to 2.8 s per crop on this Mac's CPU (ViT-L/14), against about 0.3 s for SpeciesNet's classifier. A per-site classifier file is 1.2 GB because the head is baked into the encoder. BioCLIP's text tower is not in the SPARROW zoo, so building the head needs open_clip and PyTorch.

## If it goes further

1. Measure properly first: a few hundred labelled camera-trap images from a public set that BioCLIP was not trained on, across regions, scored at species level and at genus level, with GBIF limits as here.
2. If it holds up, do not ship a 1.2 GB model per site. Studio already runs ONNX: run the zoo encoder once (one 1.2 GB download, or the 460 MB FP16 one) and keep a table of BioCLIP text embeddings per species (768 numbers each, about 1.5 KB in FP16), made once offline. The head is then a dot product over the species GBIF lists near each camera. That also needs no SPARROW-Engine at run time, so the ONNX Runtime version clash that stopped linking it does not arise.
3. Use it as a second opinion next to SpeciesNet (for example when SpeciesNet stops at family level or says blank), shown as such in review, not as a replacement.

Reproduce: run Studio on data/videos (prep.py fetches them), then `data/.venv/bin/python bioclip_check.py` (sparrow-engine, open_clip_torch, torch, onnx).
