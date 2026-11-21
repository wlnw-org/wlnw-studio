import { ask } from "@tauri-apps/plugin-dialog";
import { api, type RemoveCounts, type RemoveTarget } from "./api";
import { t } from "./i18n";

/** Shows what removing `target` takes out of the project, asks, and removes it. Returns what went, or null when the
 * person said no. Only Studio's records go; the audio files stay on disk. */
export async function removeFromProject(what: string, target: RemoveTarget): Promise<RemoveCounts | null> {
  const c = await api.removePreview(target);
  const lines = [
    t("Recordings: {n}", { n: c.recordings.toLocaleString() }),
    ...(c.images ? [t("Camera-trap images: {n}", { n: c.images.toLocaleString() })] : []),
    t("Detections: {n}", { n: c.detections.toLocaleString() }),
    ...(c.reviews ? [t("Your review answers: {n}", { n: c.reviews.toLocaleString() })] : []),
  ];
  const msg = [
    t("Remove {what} from this project?", { what }),
    lines.join("\n"),
    t("The files stay on your disk. Studio only forgets them: add the folder again to bring them back, then process them again."),
    ...(c.clips ? [t("Clips Studio fetched from the station are deleted from the project's clips folder. Import the station again to fetch them again.")] : []),
    t("This cannot be undone."),
  ].join("\n\n");
  const ok = await ask(msg, { title: t("Remove from project"), kind: "warning", okLabel: t("Remove"), cancelLabel: t("Cancel") });
  if (!ok) return null;
  return api.removeFromProject(target);
}

/** One line saying what was removed. */
export const removedNote = (c: RemoveCounts) =>
  t("Removed from the project: {recordings} recordings and {detections} detections. The files are still on your disk.", { recordings: c.recordings.toLocaleString(), detections: c.detections.toLocaleString() });
