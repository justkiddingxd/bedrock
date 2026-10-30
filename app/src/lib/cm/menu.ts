/** The editor's right-click menu: formatting, blocks, due dates and reminders, links, clipboard. */

import type { EditorView } from '@codemirror/view';
import { openUrl } from '@tauri-apps/plugin-opener';
import type { MenuEntry } from '../Menu.svelte';
import { addDays, localDate } from '../dates';
import { nextWeekday } from '../quick';
import {
  TABLE,
  copy,
  insertBlock,
  insertCodeBlock,
  insertLink,
  insertWikilink,
  isWrapped,
  lineKind,
  paste,
  selectAll,
  setBlock,
  setDone,
  setHeading,
  setMarker,
  toggleWrap,
} from './format';
import { toggleTask } from './wikilinks';

interface Context {
  onOpenLink: (raw: string) => void;
  onError: (message: string) => void;
}

/** A link at `pos`: a [[note]] or a web address. */
function linkAt(view: EditorView, pos: number): { note?: string; url?: string } | null {
  const line = view.state.doc.lineAt(pos);
  const at = pos - line.from;
  for (const m of line.text.matchAll(/\[\[([^\]\n]+)\]\]/g)) {
    if (at >= m.index && at <= m.index + m[0].length) return { note: m[1] };
  }
  for (const m of line.text.matchAll(/https?:\/\/[^\s)\]>]+/g)) {
    if (at >= m.index && at <= m.index + m[0].length) return { url: m[0].replace(/[.,;:!?]+$/, '') };
  }
  return null;
}

const pad = (n: number) => String(n).padStart(2, '0');
const stamp = (d: Date) => `${localDate(d)} ${pad(d.getHours())}:${pad(d.getMinutes())}`;

function reminders(): [string, string][] {
  const now = new Date();
  const inHour = new Date(now.getTime() + 60 * 60_000);
  inHour.setMinutes(Math.ceil(inHour.getMinutes() / 5) * 5, 0, 0);
  const evening = new Date(now);
  evening.setHours(19, 0, 0, 0);
  if (evening <= now) evening.setDate(evening.getDate() + 1);
  const morning = new Date(now);
  morning.setDate(morning.getDate() + 1);
  morning.setHours(9, 0, 0, 0);
  return [
    ['Через час', stamp(inHour)],
    [evening.getDate() === now.getDate() ? 'Вечером' : 'Завтра вечером', stamp(evening)],
    ['Завтра в 9', stamp(morning)],
  ];
}

export function editorMenu(view: EditorView, pos: number | null, ctx: Context): MenuEntry[] {
  const { state } = view;
  const line = state.doc.lineAt(state.selection.main.head);
  const kind = lineKind(line.text);
  const hasSelection = state.selection.ranges.some((r) => !r.empty);
  const run = (f: (v: EditorView) => unknown) => () => f(view);
  const items: MenuEntry[] = [];

  const link = pos === null ? null : linkAt(view, pos);
  if (link?.note) items.push({ label: `Открыть «${link.note.split('|')[0].split('#')[0]}»`, icon: 'doc', action: () => ctx.onOpenLink(link.note!) }, '-');
  if (link?.url) items.push({ label: 'Открыть в браузере', icon: 'external', action: () => openUrl(link.url!).catch((e) => ctx.onError(String(e))) }, '-');

  items.push(
    {
      row: [
        { icon: 'bold', title: 'Жирный (Ctrl+B)', active: isWrapped(state, '**'), action: run((v) => toggleWrap(v, '**')) },
        { icon: 'italic', title: 'Курсив (Ctrl+I)', active: isWrapped(state, '_'), action: run((v) => toggleWrap(v, '_')) },
        { icon: 'strike', title: 'Зачёркнутый', active: isWrapped(state, '~~'), action: run((v) => toggleWrap(v, '~~')) },
        { icon: 'code', title: 'Код в строке', active: isWrapped(state, '`'), action: run((v) => toggleWrap(v, '`')) },
        { icon: 'link', title: 'Ссылка на сайт', action: run(insertLink) },
        { icon: 'wikilink', title: 'Ссылка на заметку [[ ]]', action: run(insertWikilink) },
      ],
    },
    {
      row: [1, 2, 3].map((n) => ({ text: `H${n}`, title: `Заголовок ${n}`, active: kind.heading === n, action: run((v) => setHeading(v, n)) })),
    },
    { label: 'Список', icon: 'list', hint: kind.block === 'bullet' ? '✓' : undefined, action: run((v) => setBlock(v, 'bullet')) },
    { label: 'Нумерованный список', icon: 'list-ordered', hint: kind.block === 'number' ? '✓' : undefined, action: run((v) => setBlock(v, 'number')) },
    kind.block === 'task'
      ? { label: kind.done ? 'Вернуть в работу' : 'Отметить выполненной', icon: 'check', hint: 'Ctrl+L', action: run((v) => setDone(v, !kind.done)) }
      : { label: 'Задача', icon: 'check', hint: 'Ctrl+L', action: run(toggleTask) },
    { label: 'Цитата', icon: 'quote', hint: kind.block === 'quote' ? '✓' : undefined, action: run((v) => setBlock(v, 'quote')) },
    { label: 'Блок кода', icon: 'code-block', action: run(insertCodeBlock) },
    { label: 'Таблица', icon: 'table', action: run((v) => insertBlock(v, TABLE)) },
    { label: 'Разделитель', icon: 'minus', action: run((v) => insertBlock(v, '---')) },
  );

  const today = localDate();
  if (kind.block === 'task') {
    items.push('-', { heading: 'Срок' }, {
      row: [
        ['Сегодня', today],
        ['Завтра', addDays(today, 1)],
        ['В пн', nextWeekday(today, 1)],
        ['Через неделю', addDays(today, 7)],
      ].map(([text, day]) => ({ text, title: `Срок: ${day}`, action: run((v) => setMarker(v, 'due', day)) })),
    });
  }
  if (kind.block !== 'task') items.push('-');
  items.push(
    { heading: 'Напомнить' },
    { row: reminders().map(([text, at]) => ({ text, title: `Напоминание: ${at}`, action: run((v) => setMarker(v, 'remind', at)) })) },
  );

  items.push(
    '-',
    {
      label: 'Вырезать',
      icon: 'cut',
      hint: 'Ctrl+X',
      action: () => (hasSelection ? copy(view, true).catch(() => ctx.onError('Не удалось вырезать: нажмите Ctrl+X')) : undefined),
    },
    {
      label: 'Копировать',
      icon: 'copy',
      hint: 'Ctrl+C',
      action: () => (hasSelection ? copy(view, false).catch(() => ctx.onError('Не удалось скопировать: нажмите Ctrl+C')) : undefined),
    },
    { label: 'Вставить', icon: 'paste', hint: 'Ctrl+V', action: () => paste(view).catch(() => ctx.onError('Вставка из меню недоступна: нажмите Ctrl+V')) },
    { label: 'Выделить всё', icon: 'select-all', hint: 'Ctrl+A', action: run(selectAll) },
  );
  return items;
}
