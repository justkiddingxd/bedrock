/** Dates are local calendar days written as YYYY-MM-DD. */

const pad = (n: number) => String(n).padStart(2, '0');

export function localDate(d = new Date()): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export function toDate(day: string): Date {
  const [y, m, d] = day.split('-').map(Number);
  return new Date(y, m - 1, d);
}

export function addDays(day: string, n: number): string {
  const d = toDate(day);
  d.setDate(d.getDate() + n);
  return localDate(d);
}

/** Monday of the week containing `day`. */
export function weekStart(day: string): string {
  const d = toDate(day);
  return addDays(day, -((d.getDay() + 6) % 7));
}

export function daysBetween(from: string, to: string): number {
  return Math.round((toDate(to).getTime() - toDate(from).getTime()) / 86_400_000);
}

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
const fmtWeekday = new Intl.DateTimeFormat('ru', { weekday: 'long' });
const fmtWeekdayShort = new Intl.DateTimeFormat('ru', { weekday: 'short' });
const fmtDayMonth = new Intl.DateTimeFormat('ru', { day: 'numeric', month: 'long' });
const fmtDayMonthYear = new Intl.DateTimeFormat('ru', { day: 'numeric', month: 'long', year: 'numeric' });
const fmtShort = new Intl.DateTimeFormat('ru', { day: 'numeric', month: 'short' });
const fmtTime = new Intl.DateTimeFormat('ru', { hour: '2-digit', minute: '2-digit' });

/** "Четверг" */
export function weekdayName(day: string): string {
  return cap(fmtWeekday.format(toDate(day)));
}

/** "Пн" */
export function weekdayShort(day: string): string {
  return cap(fmtWeekdayShort.format(toDate(day)));
}

/** "29 октября", with the year when it is not the current one. */
export function dayMonth(day: string): string {
  const d = toDate(day);
  return (d.getFullYear() === new Date().getFullYear() ? fmtDayMonth : fmtDayMonthYear).format(d);
}

/** "пт, 30 октября" */
export function dayLabel(day: string): string {
  return `${fmtWeekdayShort.format(toDate(day))}, ${dayMonth(day)}`;
}

/** "сегодня", "завтра", "вчера" or "27 окт." */
export function dueLabel(day: string, today: string): string {
  const diff = daysBetween(today, day);
  if (diff === 0) return 'сегодня';
  if (diff === 1) return 'завтра';
  if (diff === -1) return 'вчера';
  return fmtShort.format(toDate(day));
}

/** File times: "14:20" today, "вчера", "27 окт.", or a full date. */
export function relTime(ms: number): string {
  const d = new Date(ms);
  const today = localDate();
  const day = localDate(d);
  if (day === today) return fmtTime.format(d);
  if (day === addDays(today, -1)) return 'вчера';
  if (d.getFullYear() === new Date().getFullYear()) return fmtShort.format(d);
  return d.toLocaleDateString('ru');
}

export function timeOf(ms: number): string {
  return fmtTime.format(new Date(ms));
}
