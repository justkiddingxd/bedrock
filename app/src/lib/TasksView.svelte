<script lang="ts">
  import Icon from './Icon.svelte';
  import TaskRow from './TaskRow.svelte';
  import { dailyPath, plural, type TaskItem } from './api';
  import { droppable, type Dnd } from './drag';
  import { addDays, dayLabel } from './dates';
  import { parseQuickTask } from './quick';
  import { reminderLabel } from './reminder';

  interface Props {
    tasks: TaskItem[];
    today: string;
    dnd: Dnd;
    onToggle: (t: TaskItem) => void;
    onOpen: (path: string, line?: number) => void;
    onReschedule: (list: TaskItem[], date: string | null) => void;
    onTaskMenu: (e: MouseEvent, t: TaskItem) => void;
    onAddTask: (text: string, date: string) => void;
  }
  let p: Props = $props();

  let showDone = $state(false);
  let draft = $state('');
  const parsed = $derived(draft.trim() ? parseQuickTask(draft, p.today) : null);

  interface Group {
    key: string;
    title: string;
    sub?: string;
    danger?: boolean;
    items: TaskItem[];
    /** Tasks dropped on the group get this due date (`null`: none); absent: no drop. */
    date?: string | null;
  }

  const groups = $derived.by(() => {
    const today = p.today;
    const daily = dailyPath(today);
    const open = p.tasks.filter((t) => !t.done);
    const byDue = (a: TaskItem, b: TaskItem) => (a.due ?? '').localeCompare(b.due ?? '');
    const out: Group[] = [
      { key: 'overdue', title: 'Просрочено', danger: true, items: open.filter((t) => t.due && t.due < today).sort(byDue) },
      {
        key: 'today',
        title: 'Сегодня',
        sub: dayLabel(today),
        date: today,
        items: open.filter((t) => t.due === today || t.scheduled === today || (!t.due && t.path === daily)),
      },
      { key: 'tomorrow', title: 'Завтра', sub: dayLabel(addDays(today, 1)), date: addDays(today, 1), items: open.filter((t) => t.due === addDays(today, 1)) },
    ];
    for (let i = 2; i <= 7; i++) {
      const day = addDays(today, i);
      const label = dayLabel(day);
      out.push({ key: day, title: label.charAt(0).toUpperCase() + label.slice(1), date: day, items: open.filter((t) => t.due === day) });
    }
    out.push(
      { key: 'later', title: 'Позже', items: open.filter((t) => t.due && t.due > addDays(today, 7)).sort(byDue) },
      { key: 'nodate', title: 'Без срока', date: null, items: open.filter((t) => !t.due && t.path !== daily) },
    );
    if (showDone) {
      const done = p.tasks
        .filter((t) => t.done)
        .sort((a, b) => (b.doneDate ?? '').localeCompare(a.doneDate ?? ''))
        .slice(0, 50);
      out.push({ key: 'done', title: 'Выполнено', items: done });
    }
    return out.filter((g) => g.items.length > 0);
  });
  const openCount = $derived(p.tasks.filter((t) => !t.done).length);

  function submit(e: SubmitEvent) {
    e.preventDefault();
    const text = draft.trim();
    if (!text) return;
    p.onAddTask(text, p.today);
    draft = '';
  }
</script>

<div class="tasks-view">
  <div class="list-head">
    <h1 class="list-title">Предстоящие<span class="list-count">{openCount} {plural(openCount, ['открытая', 'открытые', 'открытых'])}</span></h1>
    <div class="list-tools">
      <label><input type="checkbox" class="check" bind:checked={showDone} />Показывать выполненные</label>
    </div>
  </div>

  <form class="add-task" onsubmit={submit}>
    <Icon name="plus" />
    <input bind:value={draft} placeholder="Новая задача, например «счёт бухгалтеру до 5.11 !»" aria-label="Новая задача" />
  </form>
  {#if parsed && (parsed.due || parsed.high || parsed.remind || parsed.repeat)}
    <div class="quick-hint">
      {#if parsed.due}<span>срок: {dayLabel(parsed.due)}</span>{/if}
      {#if parsed.high}<span class="high">важно</span>{/if}
            {#if parsed.remind}<span>напомню {reminderLabel(parsed.remind, p.today)}</span>{/if}
        {#if parsed.repeat}<span>🔁 {parsed.repeat}</span>{/if}
    </div>
  {/if}

  {#each groups as g (g.key)}
    <section class="task-group" use:droppable={g.date === undefined ? null : p.dnd.onDay(g.date)}>
    <h2 class="group-title" class:danger={g.danger}>
      {g.title}
      {#if g.sub}<span class="sub">{g.sub}</span>{/if}
      <span class="sub">{g.items.length}</span>
      {#if g.key === 'overdue'}
        <button class="link-btn" onclick={() => p.onReschedule(g.items, p.today)}>Перенести на сегодня</button>
      {/if}
    </h2>
    {#each g.items as t (`${t.path}:${t.line}`)}
      <TaskRow
        task={t}
        today={p.today}
        showDate={g.key === 'overdue' || g.key === 'later' || g.key === 'done'}
        onToggle={p.onToggle}
        onOpen={p.onOpen}
        onMenu={p.onTaskMenu}
      />
    {/each}
    </section>
  {/each}

  {#if groups.length === 0}
    <div class="empty">Задач нет. Напишите в любой заметке<br /><code>- [ ] задача 📅 {p.today}</code></div>
  {/if}
</div>
