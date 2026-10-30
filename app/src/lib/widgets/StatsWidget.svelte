<script lang="ts">
  import type { NoteSummary, TaskItem } from '../api';
  import { addDays, dayLabel, localDate, weekStart, weekdayShort } from '../dates';

  let { today, tasks, summaries }: { today: string; tasks: TaskItem[]; summaries: NoteSummary[] } = $props();

  const start = $derived(weekStart(today));
  const days = $derived(Array.from({ length: 7 }, (_, i) => addDays(start, i)));
  const doneByDay = $derived(days.map((d) => tasks.filter((t) => t.done && t.doneDate === d).length));
  const done = $derived(doneByDay.reduce((a, b) => a + b, 0));
  const left = $derived(tasks.filter((t) => !t.done && t.due && t.due >= today && t.due <= days[6]).length);
  const overdue = $derived(tasks.filter((t) => !t.done && t.due && t.due < today).length);
  const edited = $derived(summaries.filter((s) => localDate(new Date(s.mtime)) >= start).length);
  const top = $derived(Math.max(1, ...doneByDay));
</script>

<div class="stats">
  <div class="stat"><b>{done}</b><span>сделано</span></div>
  <div class="stat"><b>{left}</b><span>осталось на неделе</span></div>
  <div class="stat" class:danger={overdue > 0}><b>{overdue}</b><span>просрочено</span></div>
  <div class="stat"><b>{edited}</b><span>заметок изменено</span></div>
</div>
<div class="bars" role="img" aria-label="Выполнено задач по дням недели: {doneByDay.join(', ')}">
  {#each days as d, i (d)}
    <div class="bar-col" title="{dayLabel(d)}: {doneByDay[i]}">
      <div class="bar-track"><div class="bar" class:is-today={d === today} style:height="{(doneByDay[i] / top) * 100}%"></div></div>
      <span class:is-today={d === today}>{weekdayShort(d)}</span>
    </div>
  {/each}
</div>
