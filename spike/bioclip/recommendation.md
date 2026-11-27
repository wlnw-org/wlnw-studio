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
