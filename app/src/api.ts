import { invoke } from "@tauri-apps/api/core";

export interface Deployment { id: number; name: string; folder: string; serial: string | null; model: string | null; lat: number | null; lon: number | null; tz_offset_min: number; tz_known: boolean; files: number; done: number; hours: number; models: string[]; perch_region: boolean }
export interface Progress { jobs_done: number; jobs_total: number; current_model: string | null; files_done: number; files_total: number; hours_done: number; bytes_done: number; bytes_total: number; detections: number; speed_x: number; eta_s: number | null; current: string | null; errors: number }
export interface DatasetInfo { title: string; institution: string | null; people: string[]; license: string; description: string | null; contact_email: string | null }
export interface ModelInfo { id: string; name: string; version: string; licence: string; licence_url: string; citation: string; sample_rate: number; window: number; embeds: boolean; location_filter: boolean; speed_x: number }
export interface WncInfo { trained_at: string; classes: [string, string, number, number | null][] }
export interface Summary { path: string; name: string; deployments: Deployment[]; progress: Progress; running: boolean; dataset: DatasetInfo; models: ModelInfo[]; model_errors: string[]; chosen: string[]; wnc: WncInfo | null }
export interface Similar { window_id: number; file_id: number; path: string; start_s: number; end_s: number; similarity: number; site: string }
export interface TrainReport { classes: [string, string, number, number, number | null][]; skipped: [string, number][]; detections: number }
export interface SpeciesRow { label: number; model: string; scientific: string; common: string; detections: number; kept: number; reviewed: number; yes: number; no: number; threshold: number; threshold_source: string; precision: number | null; max_conf: number }
export interface Detection { id: number; file_id: number; path: string; deployment_id: number; start_s: number; end_s: number; label: number; scientific: string; common: string; model: string; confidence: number; in_range: boolean; utc: string | null; lat: number | null; lon: number | null; site: string; verdict: string | null }
export interface Clip { wav: string; spec: string; width: number; height: number; offset_s: number; window_s: number; total_s: number; max_khz: number }
export interface ScanReport { found: number; added: number; without_time: number; deployments: number }
export interface PublishReport { sent: number; added: number; duplicates: number; left_out: number; status: string | null }

export interface CatalogItem { id: string; name: string; description: string; licence: string; licence_url: string; downloadable: boolean; note: string; installed: boolean; downloading: boolean; size: number }
export interface DownloadEvent { id: string; done: number; total: number; finished: boolean; error: string | null }

export const api = {
  modelCatalog: () => invoke<CatalogItem[]>("model_catalog"),
  downloadModel: (id: string) => invoke<void>("download_model", { id }),
  cancelDownload: () => invoke<void>("cancel_download"),
  addModelsFolder: (folder: string) => invoke<string[]>("add_models_folder", { folder }),
  openProject: (file: string) => invoke<void>("open_project", { file }),
  summary: () => invoke<Summary>("summary"),
  scanFolder: (folder: string) => invoke<ScanReport>("scan_folder", { folder }),
  updateDeployment: (id: number, name: string, lat: number | null, lon: number | null, tzHours: number) => invoke<void>("update_deployment", { id, name, lat, lon, tzHours }),
  setSetting: (key: string, value: string) => invoke<void>("set_setting", { key, value }),
  startRun: () => invoke<void>("start_run"),
  stopRun: () => invoke<void>("stop_run"),
  species: () => invoke<SpeciesRow[]>("species"),
  reviewQueue: (label: number, model: string, n: number) => invoke<Detection[]>("review_queue", { label, model, n }),
  clip: (file: string, startS: number, endS: number) => invoke<Clip>("clip", { file, startS, endS }),
  nameSound: (detection: number, scientific: string, common: string, present: boolean) => invoke<boolean>("name_sound", { detection, scientific, common, present }),
  similar: (detection: number, n: number) => invoke<Similar[]>("similar", { detection, n }),
  labelWindow: (window: number, scientific: string, common: string, present: boolean) => invoke<void>("label_window", { window, scientific, common, present }),
  trainWnc: () => invoke<TrainReport>("train_wnc"),
  wncExport: (out: string) => invoke<void>("wnc_export", { out }),
  wncImport: (file: string) => invoke<number>("wnc_import", { file }),
  setVerdict: (detection: number, verdict: string | null) => invoke<void>("set_verdict", { detection, verdict }),
  exportTo: (format: string, out: string) => invoke<number>("export_to", { format, out }),
  publishTo: (dataset: string, key: string, submit: boolean) => invoke<PublishReport>("publish_to", { dataset, key, submit }),
};

export const fmtHours = (h: number) => (h < 1 ? `${Math.round(h * 60)} min` : `${h.toFixed(h < 10 ? 1 : 0)} h`);
export const fmtBytes = (b: number) => (b > 1e12 ? `${(b / 1e12).toFixed(2)} TB` : b > 1e9 ? `${(b / 1e9).toFixed(1)} GB` : `${Math.round(b / 1e6)} MB`);
export const fmtDuration = (s: number) => (s < 90 ? `${Math.round(s)} s` : s < 5400 ? `${Math.round(s / 60)} min` : `${(s / 3600).toFixed(1)} h`);

export const MODEL_NAMES: Record<string, string> = { "birdnet-2.4": "BirdNET", "perch-2.0": "Perch", wnc: "Wildlife Network Classifier" };
export const modelName = (id: string) => MODEL_NAMES[id] ?? id;
export const modelTag = (id: string) => (id === "wnc" ? "WN Classifier" : modelName(id));
