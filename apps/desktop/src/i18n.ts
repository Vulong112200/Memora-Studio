// Bilingual UI (owner decision 2026-11-11): every string is written in both languages where it is used,
// L("English", "Tiếng Việt"). The language is a per-computer preference kept in localStorage (default Vietnamese);
// switching reloads the window so module-level lists pick it up too.
export type Lang = "vi" | "en";

export const lang: Lang = (() => {
  try {
    return localStorage.getItem("memora.lang") === "en" ? "en" : "vi";
  } catch {
    return "vi";
  }
})();

document.documentElement.lang = lang;

export const L = (en: string, vi: string) => (lang === "vi" ? vi : en);

/** Date and time in the UI language ("20/02/2026, 15:21"). */
export const dateTime = (d: Date) => d.toLocaleString(lang === "vi" ? "vi-VN" : "en-GB", { dateStyle: "short", timeStyle: "short" });

export function setLang(l: Lang) {
  try {
    localStorage.setItem("memora.lang", l);
  } catch {
    // storage blocked: the language stays as it is
  }
  location.reload();
}
