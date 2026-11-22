import { homeDir } from "@tauri-apps/api/path";

/**
 * File paths shown in messages start with "~" instead of the home folder (/Users/<name>, /home/<name> or
 * C:\Users\<name>), so the user's account name does not appear on screen, in screenshots or in screen recordings.
 * The path used for the file itself is unchanged.
 */
let home: string | null = null;

export async function initHome(): Promise<void> {
  try { home = (await homeDir()).replace(/[\\/]+$/, ""); } catch { home = null; }
}

/** "~/…" for a path inside the home folder, else the path as it is. Pure apart from the cached home folder. */
export function shortPath(p: string, homeFolder: string | null = home): string {
  if (!homeFolder || !p) return p;
  const win = /^[A-Za-z]:[\\/]/.test(homeFolder);
  const same = win ? p.toLowerCase().startsWith(homeFolder.toLowerCase()) : p.startsWith(homeFolder);
  if (!same) return p;
  const rest = p.slice(homeFolder.length);
  if (rest === "") return "~";
  return /^[\\/]/.test(rest) ? "~" + rest : p;
}
