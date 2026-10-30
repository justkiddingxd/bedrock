<script lang="ts">
  import Icon from './Icon.svelte';
  import { dailyDay, type TaskItem } from './api';
  import { draggable } from './drag';
  import { dayMonth, daysBetween, dueLabel } from './dates';
  import { reminderLabel } from './reminder';

  interface Props {
    task: TaskItem;
    today: string;
    showNote?: boolean;
    showDate?: boolean;
    onToggle: (t: TaskItem) => void;
    onOpen: (path: string, line?: number) => void;
    /** Date and other actions (button on hover, right click). */
    onMenu?: (e: MouseEvent, t: TaskItem) => void;
  }
  let { task, today, showNote = true, showDate = true, onToggle, onOpen, onMenu }: Props = $props();

  const PRIORITY: Record<string, string> = { high: 'важно', medium: 'средний', low: 'низкий' };
  /** The check pops and draws itself when the task gets done here. */
  let pop = $state(false);

  const source = $derived.by(() => {
    const day = dailyDay(task.path);
    if (!day) return task.noteTitle;
    return day === today ? 'Заметка дня' : `Дневник, ${dayMonth(day)}`;
  });
  const dueClass = $derived(
    !task.due || task.done ? '' : daysBetween(today, task.due) < 0 ? 'due-overdue' : task.due === today ? 'due-today' : '',
  );
</script>

<div
  class="task-row"
  class:done={task.done}
  role="listitem"
  use:draggable={{ kind: 'task', task }}
  oncontextmenu={(e) => {
    if (!onMenu) return;
    e.preventDefault();
    onMenu(e, task);
  }}
>
  <input
    type="checkbox"
    class="check"
    class:pop
    checked={task.done}
    aria-label="Выполнено: {task.text}"
    onclick={(e) => {
      e.preventDefault();
      if (!task.done) {
        pop = true;
        setTimeout(() => (pop = false), 500);
      }
      onToggle(task);
    }}
  />
  <div class="task-main">
    <button class="task-text" title="Открыть в заметке" onclick={() => onOpen(task.path, task.line)}>{task.text}</button>
    <div class="task-sub">
      {#if showNote}<button class="note-link" onclick={() => onOpen(task.path)}>{source}</button>{/if}
      {#if showDate && task.due}<span class={dueClass}>{dueLabel(task.due, today)}</span>{/if}
      {#if task.reminder && !task.done}
        <span class="task-alarm" title="Напоминание">
          <Icon name="bell" size={12} />{task.reminder.slice(0, 10) === task.due || task.reminder.slice(0, 10) === today
            ? task.reminder.slice(11)
            : reminderLabel(task.reminder, today)}
        </span>
      {/if}
      {#if task.recurrence && !task.done}
        <span class="task-repeat" title="Повторяется"><Icon name="repeat" size={12} />{task.recurrence}</span>
      {/if}
      {#if task.status === '/' && !task.done}<span class="task-progress">в работе</span>{/if}
      {#if PRIORITY[task.priority] && !task.done}<span class="prio-{task.priority}">{PRIORITY[task.priority]}</span>{/if}
      {#if task.done && task.doneDate}<span>выполнено {dueLabel(task.doneDate, today)}</span>{/if}
    </div>
  </div>
  {#if onMenu && !task.done}
    <button class="row-action" title="Срок и действия" aria-label="Срок и действия: {task.text}" onclick={(e) => onMenu(e, task)}>
      <Icon name="calendar" size={14} />
    </button>
  {/if}
</div>
