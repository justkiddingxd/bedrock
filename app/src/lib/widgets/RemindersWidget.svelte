<script lang="ts">
  import Icon from '../Icon.svelte';
  import { api, displayName, type ReminderItem, type TaskItem } from '../api';
  import { addDays, dayMonth, localStamp } from '../dates';

  interface Props {
    today: string;
    /** Changes whenever the notes do, so the list is read again. */
    tasks: TaskItem[];
    days: number;
    /** Repeating ones show even when their next time is further away. */
    repeating: boolean;
    onOpen: (path: string, line?: number) => void;
    onEdit: (r: ReminderItem) => void;
    onNew: () => void;
  }
  let p: Props = $props();

  let items = $state<ReminderItem[]>([]);
  $effect(() => {
    void p.tasks;
    api
      .listReminders()
      .then((r) => (items = r))
      .catch(() => {});
  });

  const now = $derived.by(() => {
    void p.tasks;
    return localStamp();
  });
  const last = $derived(addDays(p.today, p.days - 1));
  const when = (at: string) => {
    const [day, time] = at.split(' ');
    if (day === p.today) return time;
    return `${day === addDays(p.today, 1) ? 'завтра' : dayMonth(day)}, ${time}`;
  };
  const near = (r: ReminderItem) => r.at.slice(0, 10) >= p.today && r.at.slice(0, 10) <= last;
  const upcoming = $derived(
    items
      .filter((r) => !r.done && (near(r) || (p.repeating && r.repeat && r.at >= now)))
      .sort((a, b) => a.at.localeCompare(b.at))
      .slice(0, 12),
  );
</script>

{#each upcoming as r (`${r.path}:${r.line}`)}
  <div class="reminder-item">
    <button
      class="next-row reminder-row"
      class:passed={!r.repeat && r.at < now}
      onclick={() => p.onOpen(r.path, r.line)}
      oncontextmenu={(e) => {
        e.preventDefault();
        p.onEdit(r);
      }}
    >
      <span class="d"><Icon name={r.repeat ? 'repeat' : 'bell'} size={12} />{when(r.at)}</span>
      <span><span class="t">{r.text || r.noteTitle}</span> <span class="src">· {r.repeat ?? displayName(r.path)}</span></span>
    </button>
    <button class="row-action" title="Изменить" aria-label="Изменить: {r.text || r.noteTitle}" onclick={() => p.onEdit(r)}><Icon name="pencil" size={14} /></button>
  </div>
{:else}
  <div class="widget-empty">
    {p.days === 1 ? 'На сегодня напоминаний нет.' : 'На ближайшие дни напоминаний нет.'} Напишите в заметке «напомнить завтра в 9» или «напоминай каждый день в 9».
  </div>
{/each}
<button class="widget-new" onclick={p.onNew}><Icon name="plus" size={12} />Новое напоминание</button>
