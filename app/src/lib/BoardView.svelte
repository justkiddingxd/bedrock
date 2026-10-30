<script lang="ts">
  import { flip } from 'svelte/animate';
  import Icon from './Icon.svelte';
  import TaskRow from './TaskRow.svelte';
  import { plural, type TaskItem } from './api';
  import { addDays } from './dates';
  import { droppable, type DropSpec } from './drag';
  import { ease, ms } from './motion';

  interface Props {
    tasks: TaskItem[];
    today: string;
    onOpen: (path: string, line?: number) => void;
    onToggle: (t: TaskItem) => void;
    /** " " to do, "/" in progress, "x" done. */
    onSetStatus: (t: TaskItem, status: string) => void;
    onTaskMenu: (e: MouseEvent, t: TaskItem) => void;
  }
  let p: Props = $props();

  /** Done tasks stay on the board this long. */
  const DONE_DAYS = 14;
  const LIMIT = 40;

  type ColumnId = 'todo' | 'doing' | 'done';
  interface Column {
    id: ColumnId;
    title: string;
    status: string;
    empty: string;
    items: TaskItem[];
  }

  let filter = $state('');
  let expanded = $state<Record<string, boolean>>({});

  const columnOf = (t: TaskItem): ColumnId => (t.done ? 'done' : t.status === '/' ? 'doing' : 'todo');
  const PRIORITY = { high: 0, medium: 1, normal: 2, low: 3 };
  /** Overdue and soon first, then by priority; tasks without a date last. */
  const byDue = (a: TaskItem, b: TaskItem) =>
    (a.due ?? '9999').localeCompare(b.due ?? '9999') || PRIORITY[a.priority] - PRIORITY[b.priority];

  const columns = $derived.by((): Column[] => {
    const q = filter.trim().toLowerCase();
    const shown = p.tasks.filter((t) => !q || `${t.text} ${t.noteTitle} ${t.tags.map((x) => `#${x}`).join(' ')}`.toLowerCase().includes(q));
    const since = addDays(p.today, -DONE_DAYS);
    return [
      { id: 'todo', title: 'Сделать', status: ' ', empty: 'Задач нет', items: shown.filter((t) => columnOf(t) === 'todo').sort(byDue) },
      {
        id: 'doing',
        title: 'В работе',
        status: '/',
        empty: 'Перетащите сюда задачу, за которую взялись',
        items: shown.filter((t) => columnOf(t) === 'doing').sort(byDue),
      },
      {
        id: 'done',
        title: 'Готово',
        status: 'x',
        empty: 'Здесь будут задачи, выполненные за две недели',
        items: shown
          .filter((t) => t.done && t.status !== '-' && (t.doneDate ?? '') >= since)
          .sort((a, b) => (b.doneDate ?? '').localeCompare(a.doneDate ?? '')),
      },
    ];
  });
  const open = $derived(columns[0].items.length + columns[1].items.length);

  const dropInto = (col: Column): DropSpec => ({
    accepts: (d) => d.kind === 'task' && columnOf(d.task) !== col.id,
    onDrop: (d) => {
      if (d.kind === 'task') p.onSetStatus(d.task, col.status);
    },
  });
</script>

<div class="board-view">
  <div class="list-head">
    <h1 class="list-title">Доска<span class="list-count">{open} {plural(open, ['открытая', 'открытые', 'открытых'])}</span></h1>
    <label class="board-filter">
      <Icon name="search" size={14} />
      <input bind:value={filter} placeholder="Текст, заметка или #метка" aria-label="Фильтр задач" spellcheck="false" />
    </label>
  </div>
  <p class="board-hint">Перетаскивайте задачи между колонками. «В работе» записывается в заметку как <code>- [/]</code>.</p>

  <div class="board">
    {#each columns as col (col.id)}
      <section class="board-col" aria-label={col.title} use:droppable={dropInto(col)}>
        <h2 class="board-col-head">
          <span class="board-dot {col.id}" aria-hidden="true"></span>{col.title}<span class="sub">{col.items.length}</span>
        </h2>
        <div class="board-cards" role="list">
          {#each expanded[col.id] ? col.items : col.items.slice(0, LIMIT) as t (`${t.path}:${t.line}:${t.done}`)}
            <div class="board-card" animate:flip={{ duration: ms(220), easing: ease }}>
              <TaskRow task={t} today={p.today} onToggle={p.onToggle} onOpen={p.onOpen} onMenu={p.onTaskMenu} />
            </div>
          {:else}
            <div class="board-empty">{col.empty}</div>
          {/each}
          {#if col.items.length > LIMIT && !expanded[col.id]}
            <button class="link-btn board-more" onclick={() => (expanded[col.id] = true)}>Показать ещё {col.items.length - LIMIT}</button>
          {/if}
        </div>
      </section>
    {/each}
  </div>
</div>
