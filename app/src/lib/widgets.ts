/** Widgets of the «Сегодня» screen: which are shown, in which column and order, and their options. */

export type WidgetId =
  | 'tasks'
  | 'daily'
  | 'next'
  | 'recent'
  | 'reminders'
  | 'important'
  | 'pinned'
  | 'inbox'
  | 'month'
  | 'stats'
  | 'capture';

export interface WidgetInfo {
  id: WidgetId;
  title: string;
  icon: string;
  about: string;
}

export const WIDGETS: WidgetInfo[] = [
  { id: 'tasks', title: 'Задачи на день', icon: 'check', about: 'задачи дня и просроченные' },
  { id: 'daily', title: 'Заметка дня', icon: 'doc', about: 'пишите прямо здесь' },
  { id: 'next', title: 'Дальше', icon: 'calendar', about: 'ближайшие задачи со сроком' },
  { id: 'reminders', title: 'Напоминания', icon: 'bell', about: 'строки с ⏰ на ближайшие дни' },
  { id: 'important', title: 'Важное', icon: 'flag', about: 'задачи с высоким приоритетом' },
  { id: 'month', title: 'Календарь', icon: 'calendar', about: 'месяц со сроками и заметками дня' },
  { id: 'stats', title: 'Неделя', icon: 'check', about: 'что сделано за неделю' },
  { id: 'capture', title: 'Быстрая запись', icon: 'pencil', about: 'мысль сразу в новую заметку' },
  { id: 'inbox', title: 'Без папки', icon: 'inbox', about: 'заметки, ещё не разложенные' },
  { id: 'pinned', title: 'Закреплённые', icon: 'pin', about: 'заметки из «Закреплено»' },
  { id: 'recent', title: 'Недавно изменённые', icon: 'notes', about: 'последние правки' },
];

export type Settings = Record<string, string | number | boolean>;

export interface Layout {
  columns: [WidgetId[], WidgetId[]];
  settings: Partial<Record<WidgetId, Settings>>;
}

export const DEFAULT_LAYOUT: Layout = {
  columns: [
    ['tasks', 'reminders', 'next'],
    ['daily', 'recent'],
  ],
  settings: {},
};

const KEY = 'bedrock.today.v1';
const KNOWN = new Set<string>(WIDGETS.map((w) => w.id));

export function loadLayout(): Layout {
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? 'null');
    if (!raw || !Array.isArray(raw.columns)) return structuredClone(DEFAULT_LAYOUT);
    // Unknown and repeated widgets are dropped, so old or edited data cannot break the screen.
    const seen = new Set<string>();
    const column = (ids: unknown): WidgetId[] =>
      (Array.isArray(ids) ? ids : []).filter((id): id is WidgetId => KNOWN.has(id) && !seen.has(id) && !!seen.add(id));
    return { columns: [column(raw.columns[0]), column(raw.columns[1])], settings: raw.settings ?? {} };
  } catch {
    return structuredClone(DEFAULT_LAYOUT);
  }
}

export function saveLayout(layout: Layout) {
  localStorage.setItem(KEY, JSON.stringify(layout));
}

/** `layout` with widget `id` placed in `column` before `before` (or at the end); also adds a hidden one. */
export function placed(layout: Layout, id: WidgetId, column: 0 | 1, before: WidgetId | null): Layout {
  const columns = layout.columns.map((c) => c.filter((x) => x !== id)) as [WidgetId[], WidgetId[]];
  const at = before ? columns[column].indexOf(before) : -1;
  columns[column].splice(at < 0 ? columns[column].length : at, 0, id);
  return { ...layout, columns };
}

export function removed(layout: Layout, id: WidgetId): Layout {
  return { ...layout, columns: layout.columns.map((c) => c.filter((x) => x !== id)) as [WidgetId[], WidgetId[]] };
}

export function withSetting(layout: Layout, id: WidgetId, key: string, value: string | number | boolean): Layout {
  return { ...layout, settings: { ...layout.settings, [id]: { ...layout.settings[id], [key]: value } } };
}

export function setting<T extends string | number | boolean>(layout: Layout, id: WidgetId, key: string, fallback: T): T {
  const v = layout.settings[id]?.[key];
  return typeof v === typeof fallback ? (v as T) : fallback;
}
