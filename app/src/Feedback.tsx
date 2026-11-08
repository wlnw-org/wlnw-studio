import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useT } from "./i18n";

function system(): string {
  const ua = navigator.userAgent;
  return /Windows/i.test(ua) ? "windows" : /Mac OS X|Macintosh/i.test(ua) ? "macos" : /Linux/i.test(ua) ? "linux" : "other";
}

/** Opens the feedback form on wlnw.org in the browser, with the app version and system filled in. Nothing is sent from the app. */
export function FeedbackLink() {
  const t = useT();
  async function go() {
    const v = await getVersion().catch(() => "");
    openUrl(`https://wlnw.org/feedback?from=studio&v=${encodeURIComponent(v)}&os=${system()}`).catch(() => {});
  }
  return <button className="link small" onClick={go} title={t("Opens a form on wlnw.org in your browser")}>{t("Send feedback or report a problem")}</button>;
}

/** Opens the how-to guides for Studio on wlnw.org. */
export function HelpLink() {
  const t = useT();
  return <button className="link small" onClick={() => openUrl("https://wlnw.org/help/studio").catch(() => {})} title={t("Opens the guides on wlnw.org in your browser")}>{t("Help and how-to guides")}</button>;
}

const GUIDES: Record<string, string> = {
  recordings: "first-project", process: "find-species", review: "review", images: "camera-traps",
  publish: "export", files: "files-history", history: "files-history",
};

/** "How this works" for the screen that is open: opens its guide on wlnw.org. */
export function ScreenHelp({ step }: { step: string }) {
  const t = useT();
  const slug = GUIDES[step];
  if (!slug) return null;
  return <button className="link small screen-help" onClick={() => openUrl(`https://wlnw.org/help/studio/${slug}`).catch(() => {})} title={t("Opens the guide for this screen on wlnw.org")}>{t("How this works")}</button>;
}

/** Next to an error: the troubleshooting guide. */
export function ErrorHelp() {
  const t = useT();
  return <button className="link inline" onClick={(e) => { e.stopPropagation(); openUrl("https://wlnw.org/help/studio/files-history#common-problems").catch(() => {}); }}>{t("Get help")}</button>;
}
