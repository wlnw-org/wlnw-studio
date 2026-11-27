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

## 2. Test set

| Source | Images | Species | Where | Notes |
|---|---|---|---|---|
| iNaturalist (testset_inat.csv) | 444 | 166 | Tamil Nadu and Kerala wetlands and coasts (waders, terns, herons, egrets, ibises, kingfishers, storks, plovers); Tamil Nadu, Kerala and Karnataka (mammals, junglefowl, peafowl, spurfowl) | Research grade, observed and uploaded on or after 2025-06-01, one photo per observation, at most 3 per species (up to 6 fetched, 3 kept at random). Licences: CC-BY-4.0 84, CC-BY-NC-4.0 359, CC0-1.0 1 |
| LILA BC (testset_lila.csv) | 224 | 75 | ENA24 (eastern USA), Orinoquia (Colombia), Desert Lion (Namibia) | From BioCLIP 2's held-out benchmark IDLE-OO Camera Traps; one image per sequence where known. Site coordinates are approximate (the label files have none). SpeciesNet was trained on LILA sets, so it may have seen these. |
| Our footage (testset_ours.csv) | 27 | 10 | a privately shared wetland video, Tamil Nadu coast (14 frames, one every 4 s, all the same common redshank); Camera survey demo, northern Italy (13 images) | Hand labels. The redshank's genus is certain and species likely; the 14 frames are one bird, so they count as one sighting in spirit. The demo images' source is not recorded and they date from 2016, so BioCLIP 2 may have seen them. |

Every image keeps its source, licence, URL, place, date and true species in its CSV. Images stay in data/ (not committed).

## 3. How

- **MegaDetector and SpeciesNet** as Studio 0.1.25 runs them: the `wlnw` command line built from this repository scanned the images, each folder got its place and country (`wlnw country`, ENA24 also state VA), and `wlnw run` ran MDV6 + SpeciesNet 4.0.3a with its geofence. The best animal box per image is used. MegaDetector found an animal in 598/695 = 86% (83 to 88) images.
- **Candidate species** per place: every mammal and up to 800 birds GBIF has records of within 1 degree (places rounded to 0.5 degree), plus 12 common domestic animals and people. 3021 species in all. The true species was in its place's list for 662/695 = 95% (93 to 97) images.
- **(b) BioCLIP 2** with open_clip and PyTorch (imageomics/bioclip-2), text prompt `a photo of <kingdom ... genus species>.`, softmax over the place's candidates with BioCLIP's logit scale, on the MegaDetector crop (box plus 10% margin) and on the whole frame.
- **(c) Zoo ONNX encoder + embedding table**: the SPARROW zoo's `bioclip-2` (FP32) and `bioclip-2-fp16` image encoders run once per image with ONNX Runtime on the CPU (as Studio runs its models), preprocessing as the zoo manifest says, then a dot product with the precomputed text embeddings (FP32 table, or FP16 table for the FP16 run) and the same softmax.
- Intervals are 95% Wilson intervals. Genus level counts a prediction right when its genus matches the true genus (GBIF backbone). iNaturalist uses newer bird names than the GBIF backbone BioCLIP's candidates come from (Botaurus sinensis is Ixobrychus sinensis there, Thinornis dubius is Charadrius dubius, Ardea coromanda is Bubulcus coromandus), so a prediction with the same family and species epithet as the truth counts as the same species, for SpeciesNet and BioCLIP alike. This changed 12 of BioCLIP's top 1 answers from wrong to right.

## 4. Results

### 4a. Accuracy

BioCLIP (b) on the crop, for images where MegaDetector found an animal; whole frame for all images. SpeciesNet is Studio's answer (top 1 only: Studio stores its other suggestions by common name).

| Set | n (crop) | BioCLIP crop top 1 species | top 5 species | top 1 genus | top 5 genus | BioCLIP whole frame top 1 species | SpeciesNet species | SpeciesNet genus |
|---|---|---|---|---|---|---|---|---|
| All | 598 | 402/598 = 67% (63 to 71) | 507/598 = 85% (82 to 87) | 461/598 = 77% (74 to 80) | 525/598 = 88% (85 to 90) | 469/695 = 67% (64 to 71) | 165/695 = 24% (21 to 27) | 197/695 = 28% (25 to 32) |
| Birds | 275 | 196/275 = 71% (66 to 76) | 243/275 = 88% (84 to 92) | 229/275 = 83% (78 to 87) | 253/275 = 92% (88 to 95) | 236/314 = 75% (70 to 80) | 13/314 = 4% (2 to 7) | 22/314 = 7% (5 to 10) |
| Mammals | 323 | 206/323 = 64% (58 to 69) | 264/323 = 82% (77 to 86) | 232/323 = 72% (67 to 76) | 272/323 = 84% (80 to 88) | 233/381 = 61% (56 to 66) | 152/381 = 40% (35 to 45) | 175/381 = 46% (41 to 51) |
| iNaturalist | 359 | 263/359 = 73% (68 to 78) | 322/359 = 90% (86 to 92) | 300/359 = 84% (79 to 87) | 334/359 = 93% (90 to 95) | 340/444 = 77% (72 to 80) | 34/444 = 8% (6 to 11) | 47/444 = 11% (8 to 14) |
| LILA camera traps | 212 | 121/212 = 57% (50 to 64) | 158/212 = 75% (68 to 80) | 134/212 = 63% (57 to 69) | 164/212 = 77% (71 to 82) | 111/224 = 50% (43 to 56) | 121/224 = 54% (47 to 60) | 140/224 = 62% (56 to 69) |
| Our footage | 27 | 18/27 = 67% (48 to 81) | 27/27 = 100% (88 to 100) | 27/27 = 100% (88 to 100) | 27/27 = 100% (88 to 100) | 18/27 = 67% (48 to 81) | 10/27 = 37% (22 to 56) | 10/27 = 37% (22 to 56) |

The same on the crop with route (c), the zoo ONNX encoder and the embedding table:

| Set | n | (c) FP32 top 1 species | (c) FP32 top 5 species | (c) FP16 top 1 species | (c) FP16 top 5 species |
|---|---|---|---|---|---|
| All | 598 | 402/598 = 67% (63 to 71) | 503/598 = 84% (81 to 87) | 403/598 = 67% (64 to 71) | 504/598 = 84% (81 to 87) |
| Birds | 275 | 200/275 = 73% (67 to 78) | 240/275 = 87% (83 to 91) | 200/275 = 73% (67 to 78) | 241/275 = 88% (83 to 91) |
| Mammals | 323 | 202/323 = 63% (57 to 68) | 263/323 = 81% (77 to 85) | 203/323 = 63% (57 to 68) | 263/323 = 81% (77 to 85) |

### 4b. As a second opinion

BioCLIP's answer (crop when there is a box, else whole frame) in the cases where SpeciesNet does not give the right species.

| SpeciesNet said | Images | BioCLIP top 1 right (species) | BioCLIP top 1 right (genus) |
|---|---|---|---|
| stopped above species | 411 | 261/411 = 64% (59 to 68) | 314/411 = 76% (72 to 80) |
| no animal box (blank) | 97 | 58/97 = 60% (50 to 69) | 67/97 = 69% (59 to 77) |
| wrong species | 22 | 13/22 = 59% (39 to 77) | 13/22 = 59% (39 to 77) |
| right | 165 | 128/165 = 78% (71 to 83) | 134/165 = 81% (75 to 86) |

In all, where SpeciesNet did not name the right species, BioCLIP's top 1 was right for 332/530 = 63% (58 to 67).

Confidently wrong: BioCLIP's top 1 is wrong although its score is at or above the threshold (all images, crop or whole frame as above).

| Threshold | Shown (coverage) | Right when shown (precision) | Confidently wrong (share of all images) | Of those, SpeciesNet also wrong or above species | Second-opinion cases only: shown, right when shown |
|---|---|---|---|---|---|
| 0.2 | 664/695 = 96% (94 to 97) | 457/664 = 69% (65 to 72) | 207/695 = 30% (27 to 33) | 175 | 505, 330/505 = 65% (61 to 69) |
| 0.3 | 627/695 = 90% (88 to 92) | 453/627 = 72% (69 to 76) | 174/695 = 25% (22 to 28) | 147 | 475, 328/475 = 69% (65 to 73) |
| 0.4 | 585/695 = 84% (81 to 87) | 437/585 = 75% (71 to 78) | 148/695 = 21% (18 to 24) | 126 | 439, 313/439 = 71% (67 to 75) |
| 0.5 | 538/695 = 77% (74 to 80) | 421/538 = 78% (75 to 82) | 117/695 = 17% (14 to 20) | 98 | 396, 298/396 = 75% (71 to 79) |
| 0.6 | 478/695 = 69% (65 to 72) | 392/478 = 82% (78 to 85) | 86/695 = 12% (10 to 15) | 74 | 347, 273/347 = 79% (74 to 83) |
| 0.7 | 427/695 = 61% (58 to 65) | 367/427 = 86% (82 to 89) | 60/695 = 9% (7 to 11) | 52 | 309, 257/309 = 83% (79 to 87) |
| 0.8 | 388/695 = 56% (52 to 59) | 345/388 = 89% (85 to 92) | 43/695 = 6% (5 to 8) | 36 | 277, 241/277 = 87% (83 to 90) |
| 0.9 | 320/695 = 46% (42 to 50) | 289/320 = 90% (87 to 93) | 31/695 = 4% (3 to 6) | 28 | 230, 202/230 = 88% (83 to 91) |

### 4c. Threshold

Lowest threshold whose precision reaches 90% on this set: **0.9**. At 0.9 BioCLIP's answer would be shown for 320/695 = 46% (42 to 50) of images, and in the second-opinion cases it would be shown for 230 of 530 and right for 202 of those. The upper end of the interval matters: check this again on a larger set before fixing it.

**Suggested: 0.8.** Mark BioCLIP's answer as likely in review at 0.8 or more: right 345/388 = 89% (85 to 92) of the time overall and 241/277 = 87% (83 to 90) in the second-opinion cases, against 0.9, which reaches 90% but gives about a sixth fewer right second opinions. Below 0.8, show it only as a list of possibilities.

### 4d. Speed and size (this Mac, Apple silicon, CPU only)

| Route | Seconds per crop (median) | Download | Model file | Peak memory | Same top 1 as (b) |
|---|---|---|---|---|---|
| (b) open_clip + PyTorch | 0.32 | about 1.6 GB (Hugging Face, image and text towers) | | not measured | (reference) |
| (c) zoo ONNX FP32 + table | 1.05 | 561 MB zip | 1217 MB | 1385 MB | 577/598 = 96% (95 to 98) |
| (c) zoo ONNX FP16 + table | 1.01 | 459 MB zip | 609 MB | 1416 MB | 575/598 = 96% (94 to 97) |

SpeciesNet in Studio for comparison: 1.11 s per image including MegaDetector and image loading.
Embedding cosine between (b) and (c) on the same crop: median 0.9994 (FP32), 0.9994 (FP16).

Species embedding table for India: GBIF lists 814 mammals, 1,931 birds, 864 reptiles and 503 amphibians for India, 4,112 species. At 768 numbers each that is 6.3 MB in FP16 (12.6 MB in FP32), plus names. The whole table for this test (3021 species, 4 continents) is 4.6 MB in FP16.

### 4e. Recommendation

**Ship it as a second opinion, behind a setting, after two more checks.** Do not let it replace SpeciesNet's answer.

- It earns its place where SpeciesNet stops short. SpeciesNet named the right species for only 24% of these images, mostly because it falls back to "bird" or "animal" on close-up iNaturalist photos it was not built for. Where it did not name the right species, BioCLIP's top 1 was right about 6 times in 10, and at a score of 0.8 or more it was shown for about half of those cases and right in 87% of them.
- On camera-trap images the two are level (LILA: BioCLIP 57% top 1 on the crop, SpeciesNet 54%, and SpeciesNet has probably seen those images in training). BioCLIP is not a better camera-trap classifier; it is a different one that also knows birds and small mammals.
- Show it in review as "Second opinion (BioCLIP 2)" with its top 3 and their scores, only for animal boxes where SpeciesNet stopped above species or disagrees, and mark it as likely when the top score is 0.8 or more. Never accept it automatically, and never run it on frames MegaDetector found empty (it always names something).
- The cheaper design works: the zoo's ONNX encoder plus a species table gives the same accuracy as open_clip (same top 1 on 96% of crops; the rest are near ties), so Studio needs no PyTorch, no text tower and no SPARROW-Engine at run time. Use the FP16 file: 459 MB to download instead of 561 MB, same answers. It does not save memory on the CPU (about 1.4 GB peak either way), and ONNX Runtime took about 1 s per crop against 0.3 s for PyTorch on the same Mac, so test ONNX Runtime's thread settings or the CoreML execution provider before building it.
- The species table is small: about 6 MB in FP16 for every mammal, bird, reptile and amphibian GBIF lists for India, so it can ship per country or be made once for the world.

Before building it:

1. A held-out camera-trap set from South Asia. None of the LILA sets used here is from India, and our own camera-trap footage is 13 images from Italy.
2. One taxonomy. SpeciesNet, iNaturalist and the GBIF backbone name some species differently (12 of BioCLIP's answers only counted as right once renamed species were matched). Map the candidate list and SpeciesNet's labels to one backbone before comparing them in review.

### Not done or not verified

- SpeciesNet on the whole frame was not run: Studio only classifies MegaDetector crops, so "whole frame" applies to BioCLIP only. SpeciesNet's top 5 was not scored: Studio stores its other suggestions by common name.
- The LILA sites' coordinates are approximate (the label files have none); ENA24's US state (Virginia) is assumed.
- The 13 Camera survey demo images date from 2016 and their source is not recorded, so BioCLIP 2 may have been trained on them. The Encyclopedia of Life part of TreeOfLife-200M has no snapshot date; the 2025-06-01 cutoff for iNaturalist is a margin, not a verified date.
- The wetland bird is labelled common redshank by me from the video; BioCLIP split its 14 frames between common redshank (6) and spotted redshank (8). Its genus is certain. The 14 frames are one bird, and the iNaturalist set has up to 3 photos per species, so the images are not fully independent.
- PyTorch's memory was not measured, and SpeciesNet's timing ran alongside the GBIF lookups, so it may be slightly slow.
- The rename rule (same family and species epithet) is a heuristic; it could in principle match two different species.


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
