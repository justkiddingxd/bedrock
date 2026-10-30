/**
 * Understands dates and priority typed in plain Russian at the end of a new
 * task: «позвонить маме завтра», «отчёт в пятницу !», «счёт до 30.10».
 */
import { addDays, toDate } from './dates';
import { parseReminder } from './reminder';

export interface QuickTask {
  text: string;
  due: string | null;
  high: boolean;
  /** «напомнить завтра в 9»: "YYYY-MM-DD HH:MM". */
  remind: string | null;
}

const WEEKDAYS: [RegExp, number][] = [
  [/^(понедельник|пн)$/, 1],
  [/^(вторник|вт)$/, 2],
  [/^(среду|среда|ср)$/, 3],
  [/^(четверг|чт)$/, 4],
  [/^(пятницу|пятница|пт)$/, 5],
  [/^(субботу|суббота|сб)$/, 6],
  [/^(воскресенье|вс)$/, 0],
];
// Word stems; "мар" comes before "ма" so «марта» is March and «мая»/«май» is May.
const MONTHS = ['янв', 'фев', 'мар', 'апр', 'ма', 'июн', 'июл', 'авг', 'сен', 'окт', 'ноя', 'дек'];

/** Next date (after `today`) falling on weekday `dow` (0 = Sunday). */
export function nextWeekday(today: string, dow: number): string {
  const diff = (dow - toDate(today).getDay() + 7) % 7 || 7;
  return addDays(today, diff);
}

function dateOf(day: number, month: number, today: string): string | null {
  const year = toDate(today).getFullYear();
  const d = new Date(year, month, day);
  if (d.getMonth() !== month) return null;
  // A date that already passed this year means next year.
  if (d < toDate(today)) d.setFullYear(year + 1);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export function parseQuickTask(input: string, today: string): QuickTask {
  const reminder = parseReminder(input.trim());
  const remind = reminder?.at ?? null;
  let text = (reminder ? reminder.text : input).trim();
  let high = false;
  if (/\s!+$/.test(text) || /^!+\s/.test(text)) {
    high = true;
    text = text.replace(/\s!+$/, '').replace(/^!+\s/, '').trim();
  }

  const rules: [RegExp, (m: RegExpMatchArray) => string | null][] = [
    [/\s(?:на\s|к\s)?сегодня$/i, () => today],
    [/\s(?:на\s|к\s)?послезавтра$/i, () => addDays(today, 2)],
    [/\s(?:на\s|к\s)?завтра$/i, () => addDays(today, 1)],
    [/\sчерез\s(\d{1,3})\s(?:дн|день|дня|дней)\S*$/i, (m) => addDays(today, Number(m[1]))],
    [/\sчерез\sнеделю$/i, () => addDays(today, 7)],
    [
      /\s(?:в\s|во\s|до\s|к\s)?([а-яё]{2,11})$/i,
      (m) => {
        const w = WEEKDAYS.find(([re]) => re.test(m[1].toLowerCase()));
        return w ? nextWeekday(today, w[1]) : null;
      },
    ],
    [/\s(?:до\s|к\s)?(\d{1,2})\.(\d{1,2})$/, (m) => dateOf(Number(m[1]), Number(m[2]) - 1, today)],
    [
      /\s(?:до\s|к\s)?(\d{1,2})\s([а-яё]{3,8})\.?$/i,
      (m) => {
        const month = MONTHS.findIndex((p) => m[2].toLowerCase().startsWith(p));
        return month >= 0 ? dateOf(Number(m[1]), month, today) : null;
      },
    ],
    [/\s(\d{4}-\d{2}-\d{2})$/, (m) => m[1]],
  ];

  for (const [re, toDue] of rules) {
    const m = text.match(re);
    if (!m) continue;
    const due = toDue(m);
    if (due && text.length > m[0].length) return { text: text.slice(0, -m[0].length).trim(), due, high, remind };
  }
  // A reminder alone puts the task on that day's list.
  return { text, due: remind ? remind.slice(0, 10) : null, high, remind };
}

/** The markdown line for a parsed task (without "- [ ] "). */
export function taskLine(t: QuickTask): string {
  return [t.text, t.high ? '⏫' : '', t.due ? `📅 ${t.due}` : '', t.remind ? `⏰ ${t.remind}` : ''].filter(Boolean).join(' ');
}
