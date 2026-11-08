import { useEffect, useState } from "react";
import { MESSAGES } from "./messages";

/** Interface languages. Dictionaries are keyed by the exact English text; a missing key shows the English. */
export type Lang = "en" | "fr" | "pt";
export const LANGS: [Lang, string][] = [["en", "English"], ["fr", "Français"], ["pt", "Português"]];

const KEY = "wlnw.lang";
/** Fired on window when the language changes, so components re-render. */
const CHANGED = "wlnw-lang";

const isLang = (v: unknown): v is Lang => v === "en" || v === "fr" || v === "pt";

function detect(): Lang {
  try {
    const saved = localStorage.getItem(KEY);
    if (isLang(saved)) return saved;
  } catch { /* storage unavailable */ }
  const nav = (typeof navigator !== "undefined" ? navigator.language : "").slice(0, 2).toLowerCase();
  return isLang(nav) ? nav : "en";
}

let current: Lang = detect();
if (typeof document !== "undefined") document.documentElement.lang = current;

export function getLang(): Lang { return current; }

export function setLang(lang: Lang) {
  current = lang;
  try { localStorage.setItem(KEY, lang); } catch { /* storage unavailable: applies for this session */ }
  document.documentElement.lang = lang;
  window.dispatchEvent(new Event(CHANGED));
}

export type Vars = Record<string, string | number>;

/** Translate English `text` into the active language, then fill {placeholders}. Works outside components too. */
export function t(text: string, vars?: Vars): string {
  const s = current === "en" ? text : MESSAGES[current]?.[text] ?? text;
  return vars ? s.replace(/\{(\w+)\}/g, (m: string, k: string) => (k in vars ? String(vars[k]) : m)) : s;
}

/** For components: returns t and re-renders the component when the language changes. */
export function useT(): typeof t {
  const [, setL] = useState(current);
  useEffect(() => {
    const on = () => setL(current);
    window.addEventListener(CHANGED, on);
    return () => window.removeEventListener(CHANGED, on);
  }, []);
  return t;
}

/** The active language and a re-render on change (for the language menu). */
export function useLang(): Lang {
  const [l, setL] = useState(current);
  useEffect(() => {
    const on = () => setL(current);
    window.addEventListener(CHANGED, on);
    return () => window.removeEventListener(CHANGED, on);
  }, []);
  return l;
}
