/**
 * «напомнить завтра в 9», «напомни в пятницу в 15:30», «напомнить через 2 часа»:
 * finds such a phrase in a line and turns it into the marker «⏰ 2026-10-30 15:00»
 * (the Obsidian Reminder format), which the app, the server and MCP understand.
 */
import { addDays, localDate, toDate } from './dates';

export interface ParsedReminder {
  /** The line without the phrase. */
  text: string;
  /** Local time, "YYYY-MM-DD HH:MM". */
  at: string;
}

const pad = (n: number) => String(n).padStart(2, '0');
const stamp = (day: string, h: number, m: number) => `${day} ${pad(h)}:${pad(m)}`;
const END = '(?![а-яёa-z0-9])';

const TRIGGER = new RegExp(`(^|[\\s,;(—-])(напомн(?:и|ить|ите|ишь))${END}`, 'i');
const WEEKDAYS = ['воскресенье|вс', 'понедельник|пн', 'вторник|вт', 'среду|среда|ср', 'четверг|чт', 'пятницу|пятница|пт', 'субботу|суббота|сб'];
const MONTHS = ['янв', 'фев', 'мар', 'апр', 'ма', 'июн', 'июл', 'авг', 'сен', 'окт', 'ноя', 'дек'];
const PERIODS: Record<string, number> = { утром: 9, днём: 13, днем: 13, вечером: 19 };

/** A day/month that already passed this year means next year. */
function dayOf(d: number, month: number, today: string, year?: number): string | null {
  const t = toDate(today);
  const date = new Date(year ?? t.getFullYear(), month, d);
  if (date.getMonth() !== month || date.getDate() !== d) return null;
  if (year === undefined && date < t) date.setFullYear(date.getFullYear() + 1);
  return localDate(date);
}

type Found = { day?: string; time?: [number, number]; exact?: Date };
type Rule = [RegExp, (m: RegExpExecArray, today: string, now: Date) => Found | null];

const DATE_RULES: Rule[] = [
  [new RegExp(`^\\s*(?:на\\s+|в\\s+)?сегодня${END}`, 'i'), (_, t) => ({ day: t })],
  [new RegExp(`^\\s*(?:на\\s+)?послезавтра${END}`, 'i'), (_, t) => ({ day: addDays(t, 2) })],
  [new RegExp(`^\\s*(?:на\\s+)?завтра${END}`, 'i'), (_, t) => ({ day: addDays(t, 1) })],
  [
    new RegExp(`^\\s*(?:в|во)\\s+(${WEEKDAYS.join('|')})${END}`, 'i'),
    (m, t) => {
      const dow = WEEKDAYS.findIndex((w) => w.split('|').includes(m[1].toLowerCase()));
      const diff = (dow - toDate(t).getDay() + 7) % 7 || 7;
      return { day: addDays(t, diff) };
    },
  ],
  [/^\s*(\d{4})-(\d{2})-(\d{2})(?![\d-])/, (m, t) => ({ day: dayOf(+m[3], +m[2] - 1, t, +m[1]) ?? undefined })],
  [
    /^\s*(\d{1,2})\.(\d{1,2})(?:\.(\d{4}|\d{2}))?(?![\d.:])/,
    (m, t) => {
      const year = m[3] ? (m[3].length === 2 ? 2000 + +m[3] : +m[3]) : undefined;
      return { day: dayOf(+m[1], +m[2] - 1, t, year) ?? undefined };
    },
  ],
  [
    new RegExp(`^\\s*(\\d{1,2})\\s+([а-яё]{3,8})\\.?${END}`, 'i'),
    (m, t) => {
      const month = MONTHS.findIndex((p) => m[2].toLowerCase().startsWith(p));
      return month < 0 ? null : { day: dayOf(+m[1], month, t) ?? undefined };
    },
  ],
  [
    new RegExp(`^\\s*через\\s+(?:(\\d{1,3})\\s+)?(полчаса|минут[уы]?|мин|час(?:а|ов)?|ч|дн(?:я|ей)|день|недел[юиь])${END}`, 'i'),
    (m, t, now) => {
      const n = m[1] ? +m[1] : 1;
      const unit = m[2].toLowerCase();
      if (unit === 'полчаса') return { exact: new Date(now.getTime() + 30 * 60_000) };
      if (unit.startsWith('мин')) return { exact: new Date(now.getTime() + n * 60_000) };
      if (unit.startsWith('ч')) return { exact: new Date(now.getTime() + n * 3_600_000) };
      return { day: addDays(t, unit.startsWith('недел') ? 7 * n : n) };
    },
  ],
];

const TIME_RULES: Rule[] = [
  [/^\s*(?:в\s+)?(\d{1,2})[:.](\d{2})(?!\d)/, (m) => ({ time: [+m[1], +m[2]] })],
  [
    new RegExp(`^\\s*в\\s+(\\d{1,2})(?:\\s*(утра|дня|вечера|ночи|час(?:а|ов)?|ч))?${END}(?![:.]\\d)`, 'i'),
    (m) => {
      let h = +m[1];
      const part = m[2]?.toLowerCase();
      if ((part === 'вечера' || part === 'дня') && h < 12) h += 12;
      if ((part === 'ночи' || part === 'утра') && h === 12) h = 0;
      return { time: [h, 0] };
    },
  ],
  [new RegExp(`^\\s*(утром|днём|днем|вечером)${END}`, 'i'), (m) => ({ time: [PERIODS[m[1].toLowerCase()], 0] })],
];

/** Finds a reminder phrase in `line`. `now` is the current local time. */
export function parseReminder(line: string, now = new Date()): ParsedReminder | null {
  const trigger = TRIGGER.exec(line);
  if (!trigger) return null;
  const start = trigger.index + trigger[1].length;
  let rest = line.slice(start + trigger[2].length);
  const today = localDate(now);
  const found: Found = {};

  // Date and time may come in either order: «завтра в 9», «в 9 завтра».
  for (let pass = 0; pass < 3; pass++) {
    let matched = false;
    for (const [re, fn] of [...(found.day || found.exact ? [] : DATE_RULES), ...(found.time || found.exact ? [] : TIME_RULES)]) {
      const m = re.exec(rest);
      const r = m && fn(m, today, now);
      if (!m || !r || (!r.day && !r.time && !r.exact)) continue;
      Object.assign(found, r);
      rest = rest.slice(m[0].length);
      matched = true;
      break;
    }
    if (!matched) break;
  }
  if (!found.day && !found.time && !found.exact) return null;
  if (found.time && (found.time[0] > 23 || found.time[1] > 59)) return null;

  let at: string;
  if (found.exact) {
    at = stamp(localDate(found.exact), found.exact.getHours(), found.exact.getMinutes());
  } else if (found.day) {
    const [h, m] = found.time ?? [9, 0];
    at = stamp(found.day, h, m);
  } else {
    // Only a time: today, or tomorrow if it has passed.
    const [h, m] = found.time!;
    const passed = h * 60 + m <= now.getHours() * 60 + now.getMinutes();
    at = stamp(passed ? addDays(today, 1) : today, h, m);
  }

  // Drop the separators around the phrase, but keep a list marker like "- ".
  const before = line
    .slice(0, start)
    .replace(/(\S)\s*[,;—–]\s*$/, '$1')
    .replace(/(\S)\s+-\s*$/, '$1')
    .trimEnd();
  const after = rest.replace(/^[\s,;:—–-]+/, '').replace(/^(?:что|чтобы|про|о|об)\s+/i, '');
  const text = [before, after].filter((s) => s.trim()).join(' ');
  return { text: text.replace(/(\S)\s{2,}/g, '$1 ').trimEnd(), at };
}

/** The line with its reminder phrase replaced by the marker, or `null` if it has none. */
export function convertLine(line: string, now = new Date()): string | null {
  if (/⏰/.test(line)) return null;
  const r = parseReminder(line, now);
  if (!r) return null;
  return `${r.text}${r.text && !r.text.endsWith(' ') ? ' ' : ''}⏰ ${r.at}`;
}

/** "пт, 30 октября, 15:00" style label for an "YYYY-MM-DD HH:MM" stamp. */
export function reminderLabel(at: string, today: string): string {
  const [day, time] = at.split(' ');
  if (day === today) return `сегодня в ${time}`;
  if (day === addDays(today, 1)) return `завтра в ${time}`;
  const d = toDate(day);
  const date = d.toLocaleDateString('ru', { weekday: 'short', day: 'numeric', month: 'long' });
  return `${date}, ${time}`;
}
