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
