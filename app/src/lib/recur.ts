/**
 * Repeat rules typed in plain words («каждый понедельник», «по будням»,
 * «1-го числа»), written after 🔁. The same rules as the core
 * (crates/core/src/recur.rs), which makes the next occurrence of a task.
 */
import { addDays, localDate, toDate } from './dates';

export type Rule =
  | { kind: 'days'; n: number }
  | { kind: 'workdays' }
  | { kind: 'weekday'; dow: number }
  | { kind: 'months'; n: number; day: number | null }
  | { kind: 'years'; n: number };

/** Stems in Russian and English; dow 0 is Sunday. */
const WEEKDAYS: [string, string, number][] = [
  ['понедельн', 'monday', 1],
  ['вторн', 'tuesday', 2],
  ['сред', 'wednesday', 3],
  ['четверг', 'thursday', 4],
  ['пятниц', 'friday', 5],
  ['суббот', 'saturday', 6],
  ['воскресен', 'sunday', 0],
];

export function parseRule(text: string): Rule | null {
  const t = text.trim().toLowerCase();
  const num = /(\d{1,3})/.exec(t);
  const n = num && +num[1] > 0 ? +num[1] : 1;
  const monthDay = /(\d{1,2})\s*(?:-?(?:го|е|ое))?\s*числ|on the (\d{1,2})(?:st|nd|rd|th)?/.exec(t);
  if (monthDay) {
    const day = +(monthDay[1] ?? monthDay[2]);
    return day >= 1 && day <= 31 ? { kind: 'months', n: 1, day } : null;
  }
  if (/будн|weekday|рабоч/.test(t)) return { kind: 'workdays' };
  const w = WEEKDAYS.find(([ru, en]) => t.includes(ru) || t.includes(en));
  if (w) return { kind: 'weekday', dow: w[2] };
  if (/дн|день|ежедневн|day|daily/.test(t)) return { kind: 'days', n };
  if (/недел|еженедельн|week/.test(t)) return { kind: 'days', n: 7 * n };
  if (/месяц|ежемесячн|month/.test(t)) return { kind: 'months', n, day: null };
  if (/год|лет|ежегодн|year/.test(t)) return { kind: 'years', n };
  return null;
}

/** `day` of the month `n` months after the one of `base`, or that month's last day. */
function monthDay(base: string, n: number, day: number): string {
  const b = toDate(base);
  const last = new Date(b.getFullYear(), b.getMonth() + n + 1, 0).getDate();
  return localDate(new Date(b.getFullYear(), b.getMonth() + n, Math.min(day, last)));
}

/** The first occurrence strictly after `base` ("YYYY-MM-DD"). */
export function nextAfter(rule: Rule, base: string): string {
  switch (rule.kind) {
    case 'days':
      return addDays(base, rule.n);
    case 'workdays': {
      let d = addDays(base, 1);
      while ([0, 6].includes(toDate(d).getDay())) d = addDays(d, 1);
      return d;
    }
    case 'weekday':
      return addDays(base, (rule.dow - toDate(base).getDay() + 7) % 7 || 7);
    case 'months': {
      if (rule.day === null) return monthDay(base, rule.n, toDate(base).getDate());
      const same = monthDay(base, 0, rule.day);
      return same > base && rule.n === 1 ? same : monthDay(base, rule.n, rule.day);
    }
    case 'years':
      return monthDay(base, 12 * rule.n, toDate(base).getDate());
  }
}

/** Where a new repeating task starts: today for an interval («каждые 3 дня»), else the first matching day from today. */
export function firstFrom(rule: Rule, from: string): string {
  const interval = rule.kind === 'days' || rule.kind === 'years' || (rule.kind === 'months' && rule.day === null);
  return interval ? from : nextAfter(rule, addDays(from, -1));
}

const START = /^(?:каждый|каждую|каждое|каждые|по|ежедневно|еженедельно|ежемесячно|ежегодно|every|daily|weekly|monthly|yearly|\d{1,2}-?(?:го|е)?)$/i;

/** A repeat phrase made of the first (`fromStart`) or last few words of `text`, shortest first. */
export function findRepeat(text: string, fromStart = false): { repeat: string; rule: Rule; rest: string } | null {
  const words = text.trim().split(/\s+/);
  for (let k = 1; k <= Math.min(4, words.length); k++) {
    const part = fromStart ? words.slice(0, k) : words.slice(-k);
    if (!START.test(part[0])) continue;
    const repeat = part.join(' ');
    // A number starts only «15-го числа»; «2 недели» belongs to «каждые 2 недели».
    if (/^\d/.test(part[0]) && !/числ/i.test(repeat)) continue;
    const rule = parseRule(repeat);
    if (rule) return { repeat, rule, rest: (fromStart ? words.slice(k) : words.slice(0, -k)).join(' ') };
  }
  return null;
}
