/**
 * Repeat rules typed in plain words («каждый понедельник», «по будням»,
 * «1-го числа»), written after 🔁. The same rules as the core
 * (crates/core/src/recur.rs), which decides when a reminder comes due.
 */
import { addDays, localDate, toDate } from './dates';

export type Rule =
  | { kind: 'days'; n: number }
  /** On these days of the week; 0 is Sunday. */
  | { kind: 'weekly'; days: number[] }
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
  const monthDay = /(\d{1,2})\s*(?:-?(?:го|е|ое))?\s*числ|(\d{1,2})-(?:го|е|ое)|on the (\d{1,2})(?:st|nd|rd|th)?/.exec(t);
  if (monthDay) {
    const day = +(monthDay[1] ?? monthDay[2] ?? monthDay[3]);
    return day >= 1 && day <= 31 ? { kind: 'months', n: 1, day } : null;
  }
  if (/будн|weekday|рабоч/.test(t)) return { kind: 'weekly', days: [1, 2, 3, 4, 5] };
  if (/выходн|weekend/.test(t)) return { kind: 'weekly', days: [6, 0] };
  const days = WEEKDAYS.filter(([ru, en]) => t.includes(ru) || t.includes(en)).map((w) => w[2]);
  if (days.length) return { kind: 'weekly', days };
  // Years first: «ежегодно» has «дн» in it.
  if (/год|лет|year/.test(t)) return { kind: 'years', n };
  if (/дн|день|day|daily/.test(t)) return { kind: 'days', n };
  if (/недел|week/.test(t)) return { kind: 'days', n: 7 * n };
  if (/месяц|ежемесячн|month/.test(t)) return { kind: 'months', n, day: null };
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
    case 'weekly': {
      let d = addDays(base, 1);
      while (!rule.days.includes(toDate(d).getDay())) d = addDays(d, 1);
      return d;
    }
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

/** The first time after `now` of a reminder that started at `first` ("YYYY-MM-DD HH:MM"). */
export function nextTime(rule: Rule, first: string, now: string): string {
  const [start, time] = first.split(' ');
  const date = toDate(start).getDate();
  let day = start;
  // Months and years count from the start: from the 31st, a short month does not move the later ones.
  for (let k = 1; `${day} ${time}` <= now; k++) {
    if (rule.kind === 'months' && rule.day === null) day = monthDay(start, rule.n * k, date);
    else if (rule.kind === 'years') day = monthDay(start, 12 * rule.n * k, date);
    else day = nextAfter(rule, day);
  }
  return `${day} ${time}`;
}

const START = /^(?:каждый|каждую|каждое|каждые|каждого|раз|по|ежедневно|еженедельно|ежемесячно|ежегодно|every|daily|weekly|monthly|yearly|\d{1,2}-?(?:го|е)?)$/i;
const DAY_NAME = '(?:понедельн|вторн|сред|четверг|пятниц|суббот|воскресен)[а-яё]*';
/** «по» starts a rule only with the days it names: «по пятницам», not «по работе». */
const BY_DAYS = new RegExp(`^по\\s+(?:${DAY_NAME}|будн|рабоч|выходн)`, 'i');
/** What may come after a rule and refine it: «5-го», «5 числа», «в пятницу», «и четвергам». */
const REFINE = new RegExp(`^(?:\\d{1,2}(?:-?(?:го|е|ое))?\\s+числ[оа]|\\d{1,2}-(?:го|е|ое)|(?:в|во|по|и)\\s+${DAY_NAME})(?![а-яё\\d])`, 'i');
/** What may come before «15 числа» and belongs to it. */
const LEAD = /^(?:каждое|каждого|ежемесячно|каждый месяц)$/i;

type Found = { repeat: string; rule: Rule; rest: string };

function refined(repeat: string, rest: string): Found {
  for (let m; (m = REFINE.exec(rest)) && parseRule(`${repeat} ${m[0]}`); rest = rest.slice(m[0].length).trimStart()) repeat += ` ${m[0]}`;
  return { repeat, rule: parseRule(repeat)!, rest };
}

function led(repeat: string, before: string[]): Found {
  for (const n of [1, 2]) {
    const lead = before.slice(-n).join(' ');
    if (/^\d/.test(repeat) && LEAD.test(lead)) return { repeat: `${lead} ${repeat}`, rule: parseRule(`${lead} ${repeat}`)!, rest: before.slice(0, -n).join(' ') };
  }
  return { repeat, rule: parseRule(repeat)!, rest: before.join(' ') };
}

/** A repeat phrase made of the first (`fromStart`) or last few words of `text`, shortest first. */
export function findRepeat(text: string, fromStart = false): Found | null {
  const words = text.trim().split(/\s+/);
  for (let k = 1; k <= Math.min(5, words.length); k++) {
    const part = fromStart ? words.slice(0, k) : words.slice(-k);
    if (!START.test(part[0])) continue;
    const repeat = part.join(' ');
    // A number starts only «15-го числа»; «2 недели» belongs to «каждые 2 недели».
    if (/^\d/.test(part[0]) && !/числ/i.test(repeat)) continue;
    if (/^по$/i.test(part[0]) && !BY_DAYS.test(repeat)) continue;
    if (parseRule(repeat)) return fromStart ? refined(repeat, words.slice(k).join(' ')) : led(repeat, words.slice(0, -k));
  }
  return null;
}
