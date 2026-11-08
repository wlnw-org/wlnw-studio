import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";

function system(): string {
  const ua = navigator.userAgent;
  return /Windows/i.test(ua) ? "windows" : /Mac OS X|Macintosh/i.test(ua) ? "macos" : /Linux/i.test(ua) ? "linux" : "other";
}

/** Opens the feedback form on wlnw.org in the browser, with the app version and system filled in. Nothing is sent from the app. */
export function FeedbackLink() {
  async function go() {
    const v = await getVersion().catch(() => "");
    openUrl(`https://wlnw.org/feedback?from=studio&v=${encodeURIComponent(v)}&os=${system()}`).catch(() => {});
  }
  return <button className="link small" onClick={go} title="Opens a form on wlnw.org in your browser">Send feedback or report a problem</button>;
}

/** Opens the how-to guides for Studio on wlnw.org. */
export function HelpLink() {
  return <button className="link small" onClick={() => openUrl("https://wlnw.org/help/studio").catch(() => {})} title="Opens the guides on wlnw.org in your browser">Help and how-to guides</button>;
}

const GUIDES: Record<string, string> = {
  recordings: "first-project", process: "find-species", review: "review", images: "camera-traps",
  publish: "export", files: "files-history", history: "files-history",
};

/** "How this works" for the screen that is open: opens its guide on wlnw.org. */
export function ScreenHelp({ step }: { step: string }) {
  const slug = GUIDES[step];
  if (!slug) return null;
  return <button className="link small screen-help" onClick={() => openUrl(`https://wlnw.org/help/studio/${slug}`).catch(() => {})} title="Opens the guide for this screen on wlnw.org">How this works</button>;
}

/** Next to an error: the troubleshooting guide. */
export function ErrorHelp() {
  return <button className="link inline" onClick={(e) => { e.stopPropagation(); openUrl("https://wlnw.org/help/studio/files-history#common-problems").catch(() => {}); }}>Get help</button>;
}
