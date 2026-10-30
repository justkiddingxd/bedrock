<script lang="ts">
  import Icon from '../Icon.svelte';
  import { dailyDay, type Entry, type TaskItem } from '../api';
  import { dayLabel, localDate, toDate } from '../dates';
  import { droppable, type Dnd } from '../drag';

  interface Props {
    date: string;
    today: string;
    tasks: TaskItem[];
    files: Entry[];
    dnd: Dnd;
    onSelectDate: (date: string) => void;
  }
  let p: Props = $props();

  /** Months away from the month of the selected day. */
  let shift = $state(0);
  $effect(() => {
    void p.date;
    shift = 0;
  });

  const month = $derived.by(() => {
    const d = toDate(p.date);
    return new Date(d.getFullYear(), d.getMonth() + shift, 1);
  });
  const title = $derived.by(() => {
    const name = month.toLocaleDateString('ru', { month: 'long' });
    return `${name.charAt(0).toUpperCase()}${name.slice(1)} ${month.getFullYear()}`;
  });
  const due = $derived(new Set(p.tasks.filter((t) => !t.done && t.due).map((t) => t.due)));
  const notes = $derived(new Set(p.files.map((f) => dailyDay(f.path)).filter(Boolean)));

  /** Whole weeks, Monday first, covering the month. */
  const cells = $derived.by(() => {
    const lead = (month.getDay() + 6) % 7;
    const length = new Date(month.getFullYear(), month.getMonth() + 1, 0).getDate();
    return Array.from({ length: Math.ceil((lead + length) / 7) * 7 }, (_, i) => {
      const d = new Date(month.getFullYear(), month.getMonth(), 1 - lead + i);
      const day = localDate(d);
      return { day, n: d.getDate(), other: d.getMonth() !== month.getMonth(), tasks: due.has(day), note: notes.has(day) };
    });
  });
  const WEEKDAYS = ['Пн', 'Вт', 'Ср', 'Чт', 'Пт', 'Сб', 'Вс'];
</script>

<div class="month-head">
  <button class="icon-btn" aria-label="Прошлый месяц" onclick={() => shift--}><Icon name="chevron-left" size={14} /></button>
  <span class="month-title">{title}</span>
  <button class="icon-btn" aria-label="Следующий месяц" onclick={() => shift++}><Icon name="chevron-right" size={14} /></button>
</div>
<div class="month-grid" role="grid" aria-label={title}>
  {#each WEEKDAYS as w (w)}<span class="mw">{w}</span>{/each}
  {#each cells as c (c.day)}
    <button
      class="mday"
      class:other={c.other}
      class:is-today={c.day === p.today}
      class:selected={c.day === p.date && c.day !== p.today}
      title={dayLabel(c.day)}
      use:droppable={p.dnd.onDay(c.day)}
      onclick={() => p.onSelectDate(c.day)}
    >
      {c.n}
      {#if c.tasks || c.note}<span class="mdot" class:tasks={c.tasks}></span>{/if}
    </button>
  {/each}
</div>
