# BioCLIP 2 through SPARROW-Engine: feasibility check

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
