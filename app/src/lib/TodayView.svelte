<script lang="ts">
  import { flip } from 'svelte/animate';
  import Editor from './Editor.svelte';
  import { ease, ms } from './motion';
  import Icon from './Icon.svelte';
  import TaskRow from './TaskRow.svelte';
  import type { MenuEntry } from './Menu.svelte';
  import CaptureWidget from './widgets/CaptureWidget.svelte';
  import MonthWidget from './widgets/MonthWidget.svelte';
  import RemindersWidget from './widgets/RemindersWidget.svelte';
  import StatsWidget from './widgets/StatsWidget.svelte';
  import { api, dailyPath, displayName, parentDir, type Entry, type NoteRef, type NoteSummary, type TaskItem } from './api';
  import { draggable, droppable, type Dnd, type DropSpec } from './drag';
  import { addDays, dayLabel, dayMonth, dueLabel, relTime, weekStart, weekdayName, weekdayShort, toDate } from './dates';
  import { parseQuickTask } from './quick';
  import { reminderLabel } from './reminder';
  import {
    DEFAULT_LAYOUT,
    WIDGETS,
    loadLayout,
    placed,
    removed,
    saveLayout,
    setting,
    withSetting,
    type Layout,
    type WidgetId,
  } from './widgets';

  interface Props {
    date: string;
    today: string;
    tasks: TaskItem[];
    files: Entry[];
    summaries: NoteSummary[];
    pinned: string[];
    getNotes: () => NoteRef[];
    onSelectDate: (date: string) => void;
    onToggle: (t: TaskItem) => void;
    onOpen: (path: string, line?: number) => void;
    onReschedule: (list: TaskItem[], date: string | null) => void;
    onTaskMenu: (e: MouseEvent, t: TaskItem) => void;
    onNoteMenu: (e: MouseEvent, path: string) => void;
    onMenu: (e: MouseEvent, items: MenuEntry[]) => void;
    onAddTask: (text: string, date: string) => void;
    onCapture: (text: string) => Promise<void>;
    dnd: Dnd;
    onOpenDaily: (date: string) => void;
    onOpenLink: (raw: string, from: string) => void;
    onTag: (tag: string) => void;
    onError: (message: string) => void;
  }
  let p: Props = $props();

  // ------------------------------------------------------------ layout

  let layout = $state<Layout>(loadLayout());
  function apply(next: Layout) {
    layout = next;
    saveLayout(next);
  }
  const shown = $derived(new Set(layout.columns.flat()));
  const opt = <T extends string | number | boolean>(id: WidgetId, key: string, fallback: T) => setting(layout, id, key, fallback);

  const daily = $derived(dailyPath(p.date));
  const isToday = $derived(p.date === p.today);

  function titleOf(id: WidgetId): string {
    if (id === 'tasks') return isToday ? 'Задачи на сегодня' : 'Задачи на день';
    if (id === 'daily') return isToday ? 'Заметка дня' : `Заметка за ${dayMonth(p.date)}`;
    return WIDGETS.find((w) => w.id === id)?.title ?? id;
  }

  /** The daily note editor is created anew when its widget changes column: save it and read the note again first. */
  async function reloadDaily() {
    await editor?.flush();
    source = { path: daily, content: await api.readNote(daily).catch(() => '') };
  }

  async function move(id: WidgetId, column: number, before: WidgetId | null) {
    const from = layout.columns.findIndex((c) => c.includes(id));
    if (id === 'daily' && from !== column) await reloadDaily();
    apply(placed(layout, id, column as 0 | 1, before));
  }

  const dropBefore = (column: number, target: WidgetId): DropSpec => ({
    accepts: (d) => d.kind === 'widget' && d.id !== target,
    onDrop: (d) => {
      if (d.kind === 'widget') move(d.id as WidgetId, column, target);
    },
  });
  const dropEnd = (column: number): DropSpec => ({
    accepts: (d) => d.kind === 'widget',
    onDrop: (d) => {
      if (d.kind === 'widget') move(d.id as WidgetId, column, null);
    },
  });

  /** Options of a widget, then moving and hiding it. */
  function widgetMenu(e: MouseEvent, id: WidgetId) {
    const column = layout.columns.findIndex((c) => c.includes(id));
    const list = layout.columns[column];
    const at = list.indexOf(id);
    const items: MenuEntry[] = [];
    const choice = (key: string, value: string | number | boolean, label: string, current: string | number | boolean) => ({
      label,
      hint: current === value ? '✓' : undefined,
      action: () => apply(withSetting(layout, id, key, value)),
    });
    const toggle = (key: string, label: string, fallback: boolean) => {
      const on = opt(id, key, fallback);
      return { label, hint: on ? '✓' : undefined, action: () => apply(withSetting(layout, id, key, !on)) };
    };
    const counts = (values: number[], fallback: number) => {
      items.push({ heading: 'Сколько показывать' });
      for (const n of values) items.push(choice('count', n, `${n}`, opt(id, 'count', fallback)));
    };

    if (id === 'tasks') items.push({ heading: 'Показывать' }, toggle('overdue', 'Просроченные', true), toggle('done', 'Выполненные за день', true));
    if (id === 'daily') {
      const size = opt(id, 'size', 'm');
      items.push({ heading: 'Высота' }, choice('size', 's', 'Компактно', size), choice('size', 'm', 'Обычно', size), choice('size', 'l', 'Высоко', size));
    }
    if (id === 'reminders') {
      const days = opt(id, 'days', 3);
      items.push({ heading: 'На сколько дней' }, choice('days', 1, 'Только сегодня', days), choice('days', 3, '3 дня', days), choice('days', 7, 'Неделя', days));
    }
    if (id === 'next' || id === 'important' || id === 'inbox') counts([5, 10], 5);
    if (id === 'recent') counts([5, 10, 15], 5);
    if (items.length) items.push('-');

    if (at > 0) items.push({ label: 'Выше', icon: 'chevron-up', action: () => move(id, column, list[at - 1]) });
    if (at < list.length - 1) items.push({ label: 'Ниже', icon: 'chevron-down', action: () => move(id, column, list[at + 2] ?? null) });
    items.push(
      column === 0
        ? { label: 'В правую колонку', icon: 'chevron-right', action: () => move(id, 1, null) }
        : { label: 'В левую колонку', icon: 'chevron-left', action: () => move(id, 0, null) },
      '-',
      { label: 'Скрыть виджет', icon: 'x', action: () => apply(removed(layout, id)) },
    );
    p.onMenu(e, items);
  }

  function addMenu(e: MouseEvent) {
    const hidden = WIDGETS.filter((w) => !shown.has(w.id));
    const items: MenuEntry[] = [];
    if (hidden.length) {
      items.push({ heading: 'Добавить на экран' });
      for (const w of hidden) items.push({ label: w.title, icon: w.icon, hint: w.about, action: () => add(w.id) });
      items.push('-');
    }
    items.push({ label: 'Вернуть как было', action: reset });
    p.onMenu(e, items);
  }

  async function add(id: WidgetId) {
    if (id === 'daily') await reloadDaily();
    // Into the shorter column; the daily note prefers the wide right one.
    const [a, b] = layout.columns;
    const column = id === 'daily' ? 1 : a.length <= b.length ? 0 : 1;
    apply(placed(layout, id, column, null));
  }

  async function reset() {
    await reloadDaily();
    apply(structuredClone(DEFAULT_LAYOUT));
  }

  // ------------------------------------------------------------ data

  /** Tasks of the day: due or scheduled then, or written in that day's note without a date. */
  const dayTasks = $derived.by(() => {
    const d = p.date;
    const onDay = (t: TaskItem) => t.due === d || t.scheduled === d || (!t.due && t.path === daily);
    const open = p.tasks.filter((t) => !t.done && onDay(t));
    const done = p.tasks.filter((t) => t.done && t.doneDate === d && (onDay(t) || (t.due !== null && t.due < d)));
    return [...open, ...done];
  });
  const doneCount = $derived(dayTasks.filter((t) => t.done).length);
  const visibleTasks = $derived(opt('tasks', 'done', true) ? dayTasks : dayTasks.filter((t) => !t.done));
  const overdue = $derived(
    isToday && opt('tasks', 'overdue', true)
      ? p.tasks.filter((t) => !t.done && t.due && t.due < p.today).sort((a, b) => a.due!.localeCompare(b.due!))
      : [],
  );
  const next = $derived(
    p.tasks
      .filter((t) => !t.done && t.due && t.due > p.date)
      .sort((a, b) => a.due!.localeCompare(b.due!))
      .slice(0, opt('next', 'count', 5)),
  );
  const important = $derived(
    p.tasks
      .filter((t) => !t.done && t.priority === 'high')
      .sort((a, b) => (a.due ?? '9999').localeCompare(b.due ?? '9999'))
      .slice(0, opt('important', 'count', 5)),
  );
  const recent = $derived(p.summaries.filter((s) => s.path !== daily).slice(0, opt('recent', 'count', 5)));
  const inboxAll = $derived(p.summaries.filter((s) => !s.path.includes('/')));
  const inbox = $derived(inboxAll.slice(0, opt('inbox', 'count', 5)));
  const pinnedNotes = $derived(p.pinned.map((path) => p.summaries.find((s) => s.path === path)).filter((s) => !!s));
  const DAILY_HEIGHT: Record<string, string> = { s: '120px', m: '220px', l: '420px' };

  const week = $derived.by(() => {
    const start = weekStart(p.date);
    return Array.from({ length: 7 }, (_, i) => {
      const day = addDays(start, i);
      const dow = toDate(day).getDay();
      return {
        day,
        dw: weekdayShort(day),
        dn: toDate(day).getDate(),
        weekend: dow === 0 || dow === 6,
        tasks: p.tasks.some((t) => !t.done && t.due === day),
        note: p.files.some((f) => f.path === dailyPath(day)),
      };
    });
  });

  let draft = $state('');
  let typing = $state(false);
  const parsed = $derived(draft.trim() ? parseQuickTask(draft, p.today) : null);
  let editor = $state<ReturnType<typeof Editor>>();
  let source = $state<{ path: string; content: string } | null>(null);

  // The daily note of the selected day goes into the inline editor. A day
  // without a note gets an empty editor; the file appears on the first save.
  $effect(() => {
    const path = daily;
    let stale = false;
    api
      .readNote(path)
      .catch(() => '')
      .then((content) => {
        if (!stale) source = { path, content };
      });
    return () => {
      stale = true;
    };
  });

  export function flush() {
    return editor?.flush();
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!draft.trim()) return;
    p.onAddTask(draft, p.date);
    draft = '';
  }

  const noteMenu = (path: string) => (e: MouseEvent) => {
    e.preventDefault();
    p.onNoteMenu(e, path);
  };
</script>

{#snippet noteRow(s: NoteSummary, folder: boolean)}
  <button class="file-row" class:short={!folder} use:draggable={{ kind: 'note', path: s.path }} onclick={() => p.onOpen(s.path)} oncontextmenu={noteMenu(s.path)}>
    <span class="n">{displayName(s.path)}</span>
    {#if folder}<span class="f">{parentDir(s.path) || 'Без папки'}</span>{/if}
    <span class="t">{relTime(s.mtime)}</span>
  </button>
{/snippet}

{#snippet meta(id: WidgetId)}
  {#if id === 'tasks' && dayTasks.length}
    <span class="block-meta">{doneCount} из {dayTasks.length} готово</span>
  {:else if id === 'daily'}
    <button class="link-btn" style="font-size: 12.5px;" onclick={() => p.onOpenDaily(p.date)}>Открыть полностью</button>
  {:else if id === 'inbox' && inboxAll.length}
    <span class="block-meta">{inboxAll.length}</span>
  {/if}
{/snippet}

{#snippet body(id: WidgetId)}
  {#if id === 'tasks'}
    {#each visibleTasks as t (`${t.path}:${t.line}`)}
      <TaskRow task={t} today={p.today} showDate={false} onToggle={p.onToggle} onOpen={p.onOpen} onMenu={p.onTaskMenu} />
    {/each}
    <form class="add-task" onsubmit={submit}>
      <Icon name="plus" />
      <input
        bind:value={draft}
        placeholder={isToday ? 'Добавить задачу' : 'Добавить задачу на этот день'}
        aria-label="Новая задача"
        onfocus={() => (typing = true)}
        onblur={() => (typing = false)}
      />
    </form>
    {#if parsed && (parsed.due || parsed.high || parsed.remind)}
      <div class="quick-hint">
        {#if parsed.due}<span>срок: {dayLabel(parsed.due)}</span>{/if}
        {#if parsed.high}<span class="high">важно</span>{/if}
        {#if parsed.remind}<span>напомню {reminderLabel(parsed.remind, p.today)}</span>{/if}
      </div>
    {:else if typing && !draft.trim()}
      <!-- The example lives here, where it fits even in a narrow column. -->
      <div class="quick-hint muted">Например: «позвонить маме в пятницу в 18», «отчёт завтра !»</div>
    {/if}
    {#if overdue.length}
      <div class="overdue-box">
        <div class="overdue-head">
          <span class="overdue-title">{overdue.length} {overdue.length === 1 ? 'просроченная' : 'просроченные'}</span>
          <button class="link-btn" onclick={() => p.onReschedule(overdue, p.today)}>Перенести все на сегодня</button>
        </div>
        {#each overdue as t (`${t.path}:${t.line}`)}
          <TaskRow task={t} today={p.today} onToggle={p.onToggle} onOpen={p.onOpen} onMenu={p.onTaskMenu} />
        {/each}
      </div>
    {/if}
  {:else if id === 'daily'}
    <div class="daily-box" style:--cm-min={DAILY_HEIGHT[opt('daily', 'size', 'm')] ?? DAILY_HEIGHT.m}>
      <Editor
        bind:this={editor}
        {source}
        getNotes={p.getNotes}
        onOpenLink={(raw) => p.onOpenLink(raw, daily)}
        onTag={p.onTag}
        onError={p.onError}
        placeholderText="Пишите здесь — сохраняется само. Ctrl+L — задача, @завтра — срок"
      />
    </div>
  {:else if id === 'next'}
    {#each next as t (`${t.path}:${t.line}`)}
      <button
        class="next-row"
        use:draggable={{ kind: 'task', task: t }}
        onclick={() => p.onOpen(t.path, t.line)}
        oncontextmenu={(e) => {
          e.preventDefault();
          p.onTaskMenu(e, t);
        }}
      >
        <span class="d">{dueLabel(t.due!, p.date)}</span>
        <span><span class="t">{t.text}</span> <span class="src">· {displayName(t.path)}</span></span>
      </button>
    {:else}
      <div class="widget-empty">Впереди задач со сроком нет.</div>
    {/each}
  {:else if id === 'reminders'}
    <RemindersWidget today={p.today} tasks={p.tasks} days={opt('reminders', 'days', 3)} onOpen={p.onOpen} />
  {:else if id === 'important'}
    {#each important as t (`${t.path}:${t.line}`)}
      <TaskRow task={t} today={p.today} onToggle={p.onToggle} onOpen={p.onOpen} onMenu={p.onTaskMenu} />
    {:else}
      <div class="widget-empty">Важных задач нет. Отметьте задачу знаком «!» или ⏫.</div>
    {/each}
  {:else if id === 'month'}
    <MonthWidget date={p.date} today={p.today} tasks={p.tasks} files={p.files} dnd={p.dnd} onSelectDate={p.onSelectDate} />
  {:else if id === 'stats'}
    <StatsWidget today={p.today} tasks={p.tasks} summaries={p.summaries} />
  {:else if id === 'capture'}
    <CaptureWidget onCapture={p.onCapture} />
  {:else if id === 'inbox'}
    {#each inbox as s (s.path)}
      {@render noteRow(s, false)}
    {:else}
      <div class="widget-empty">Заметок без папки нет: всё разложено.</div>
    {/each}
    {#if inbox.length}<div class="widget-hint">Перетащите заметку на папку в боковой панели, чтобы разложить.</div>{/if}
  {:else if id === 'pinned'}
    {#each pinnedNotes as s (s.path)}
      {@render noteRow(s, true)}
    {:else}
      <div class="widget-empty">Закреплённых нет. Перетащите заметку в «Закреплено» в боковой панели.</div>
    {/each}
  {:else if id === 'recent'}
    {#each recent as s (s.path)}
      {@render noteRow(s, true)}
    {:else}
      <div class="widget-empty">Заметок пока нет.</div>
    {/each}
  {/if}
{/snippet}

<div class="today">
  <div class="today-head">
    <div>
      <div class="weekday">{weekdayName(p.date)}</div>
      <h1 class="today-date">{dayMonth(p.date)}</h1>
      {#if !isToday}<button class="link-btn back-today" onclick={() => p.onSelectDate(p.today)}>Вернуться к сегодня</button>{/if}
    </div>
    <div class="week" role="group" aria-label="Неделя">
      <button class="icon-btn" aria-label="Прошлая неделя" onclick={() => p.onSelectDate(addDays(p.date, -7))}><Icon name="chevron-left" size={14} /></button>
      {#each week as d (d.day)}
        <button
          class="day"
          class:selected={d.day === p.date && d.day !== p.today}
          class:is-today={d.day === p.today}
          class:weekend={d.weekend}
          aria-current={d.day === p.today ? 'date' : undefined}
          aria-pressed={d.day === p.date}
          title={dayLabel(d.day)}
          use:droppable={p.dnd.onDay(d.day)}
          onclick={() => p.onSelectDate(d.day)}
        >
          <span class="dw">{d.dw}</span>
          <span class="dn">{d.dn}</span>
          {#if d.day !== p.today}<span class="marker" class:tasks={d.tasks} class:note={!d.tasks && d.note}></span>{/if}
        </button>
      {/each}
      <button class="icon-btn" aria-label="Следующая неделя" onclick={() => p.onSelectDate(addDays(p.date, 7))}><Icon name="chevron-right" size={14} /></button>
    </div>
  </div>

  <div class="today-grid">
    {#each layout.columns as list, column (column)}
      <div class="widget-col">
        {#each list as id (id)}
          <section
            class="block widget"
            role={id === 'tasks' || id === 'important' ? 'list' : undefined}
            use:droppable={dropBefore(column, id)}
            animate:flip={{ duration: ms(240), easing: ease }}
          >
            <div class="block-head" title="Перетащите за заголовок, чтобы переставить" use:draggable={{ kind: 'widget', id, label: titleOf(id) }}>
              <h2 class="block-title">{titleOf(id)}</h2>
              {@render meta(id)}
              <button class="icon-btn widget-more" aria-label="Настроить виджет «{titleOf(id)}»" title="Настроить" onclick={(e) => widgetMenu(e, id)}>
                <Icon name="more" size={14} />
              </button>
            </div>
            {@render body(id)}
          </section>
        {/each}
        <div class="widget-drop" use:droppable={dropEnd(column)}>Перетащите виджет сюда</div>
      </div>
    {/each}
  </div>

  <div class="widget-add">
    <button class="widget-add-btn" onclick={addMenu}><Icon name="plus" size={14} />Виджеты</button>
  </div>
</div>
