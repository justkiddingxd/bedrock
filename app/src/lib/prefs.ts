/** Appearance settings, stored per device in localStorage and applied to <html>. */

export type Theme = 'dark' | 'light' | 'system';
export type TextSize = 's' | 'm' | 'l';

export interface Prefs {
  theme: Theme;
  accent: string;
  textSize: TextSize;
  /** The tags section of the sidebar. */
  showTags: boolean;
  /** Hide Markdown marks outside the line being edited (live preview). */
  livePreview: boolean;
}

/** Accent colors, each tuned for the light and the dark theme. */
export const ACCENTS = [
  { id: 'cobalt', name: 'Кобальт', light: '#2B59C3', dark: '#7EA6F5' },
  { id: 'green', name: 'Зелёный', light: '#1F7A55', dark: '#5DBB8A' },
  { id: 'clay', name: 'Терракота', light: '#B8562A', dark: '#F0915E' },
  { id: 'graphite', name: 'Графит', light: '#1F1F1C', dark: '#E8E7E3' },
] as const;

const TEXT_SIZES: Record<TextSize, string> = { s: '15px', m: '16px', l: '17.5px' };
const KEY = 'bedrock.prefs.v2';
const DEFAULTS: Prefs = { theme: 'dark', accent: 'cobalt', textSize: 'm', showTags: true, livePreview: true };

/** Sent on window when settings change, for parts that are not styled by CSS (the editor). */
export const PREFS_EVENT = 'bedrock:prefs';

export function loadPrefs(): Prefs {
  try {
    return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(KEY) ?? '{}') };
  } catch {
    return { ...DEFAULTS };
  }
}

const systemDark = window.matchMedia('(prefers-color-scheme: dark)');
let current = loadPrefs();

let fadeTimer: ReturnType<typeof setTimeout> | undefined;

export function applyPrefs(p: Prefs) {
  current = p;
  const root = document.documentElement;
  const theme = p.theme === 'system' ? (systemDark.matches ? 'dark' : 'light') : p.theme;
  const accent = ACCENTS.find((a) => a.id === p.accent) ?? ACCENTS[0];
  // A new theme or accent fades in instead of flashing (not on startup).
  if (root.dataset.theme && (root.dataset.theme !== theme || root.dataset.accent !== accent.id)) {
    root.classList.add('theme-fade');
    clearTimeout(fadeTimer);
    fadeTimer = setTimeout(() => root.classList.remove('theme-fade'), 400);
  }
  root.dataset.accent = accent.id;
  root.dataset.theme = theme;
  root.style.setProperty('--accent', theme === 'dark' ? accent.dark : accent.light);
  root.style.setProperty('--editor-size', TEXT_SIZES[p.textSize]);
  root.dataset.tags = p.showTags ? 'on' : 'off';
}

export function savePrefs(p: Prefs) {
  localStorage.setItem(KEY, JSON.stringify(p));
  applyPrefs(p);
  window.dispatchEvent(new CustomEvent(PREFS_EVENT, { detail: p }));
}

systemDark.addEventListener('change', () => applyPrefs(current));
