<script lang="ts">
  import Icon from '../Icon.svelte';
  import { api, displayName, type ReminderItem, type TaskItem } from '../api';
  import { addDays, localDate } from '../dates';
  import { reminderLabel } from '../reminder';

  interface Props {
    today: string;
    /** Changes whenever the notes do, so the list is read again. */
    tasks: TaskItem[];
    days: number;
    onOpen: (path: string, line?: number) => void;
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
    const d = new Date();
    return `${localDate(d)} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  });
  const last = $derived(addDays(p.today, p.days - 1));
  const upcoming = $derived(
    items
      .filter((r) => !r.done && r.at.slice(0, 10) >= p.today && r.at.slice(0, 10) <= last)
      .sort((a, b) => a.at.localeCompare(b.at))
      .slice(0, 12),
  );
</script>

{#each upcoming as r (`${r.path}:${r.line}`)}
  <button class="next-row reminder-row" class:passed={r.at < now} onclick={() => p.onOpen(r.path, r.line)}>
    <span class="d"><Icon name="bell" size={12} />{r.at.slice(0, 10) === p.today ? r.at.slice(11) : reminderLabel(r.at, p.today)}</span>
    <span><span class="t">{r.text || r.noteTitle}</span> <span class="src">· {displayName(r.path)}</span></span>
  </button>
{:else}
  <div class="widget-empty">
    {p.days === 1 ? 'На сегодня напоминаний нет.' : 'На ближайшие дни напоминаний нет.'} Напишите в заметке «напомнить завтра в 9».
  </div>
{/each}
