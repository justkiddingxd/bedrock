<script lang="ts">
  import { untrack } from 'svelte';
  import Icon from './Icon.svelte';
  import { localDate, localStamp } from './dates';
  import { firstFrom, nextTime, parseRule } from './recur';
  import { reminderLabel } from './reminder';

  interface Props {
    /** What it reminds of; `null` asks for it (a new reminder). */
    text: string | null;
    at?: string | null;
    repeat?: string | null;
    onSave: (at: string, repeat: string | null, text: string) => void;
    /** Takes the reminder off the line; only for one that has it. */
    onRemove?: () => void;
    onClose: () => void;
  }
  let { text, at = null, repeat = null, onSave, onRemove, onClose }: Props = $props();

  const CHOICES: [string, string][] = [
    ['Один раз', ''],
    ['Каждый день', 'каждый день'],
    ['По будням', 'по будням'],
    ['Каждую неделю', 'каждую неделю'],
    ['Каждый месяц', 'каждый месяц'],
    ['Каждый год', 'каждый год'],
  ];

  /** The next time of the one being edited; for a new one, the next full hour. */
  function initial(): string {
    const rule = repeat ? parseRule(repeat) : null;
    if (at) return rule ? nextTime(rule, at, localStamp()) : at;
    const next = new Date();
    next.setHours(next.getHours() + 1, 0, 0, 0);
    return localStamp(next);
  }
  const start = untrack(initial);

  let day = $state(start.slice(0, 10));
  let time = $state(start.slice(11));
  // A rule of the list is a chip; any other one is typed in, and typing wins over the chip.
  const listed = untrack(() => CHOICES.some(([, value]) => value === (repeat ?? '')));
  let chosen = $state(untrack(() => (listed ? (repeat ?? '') : '')));
  let custom = $state(untrack(() => (listed ? '' : (repeat ?? ''))));
  let about = $state('');

  const wanted = $derived(custom.trim() || chosen);
  const parsed = $derived(wanted ? parseRule(wanted) : null);
  const unclear = $derived(!!wanted && !parsed);
  /** A repeating one starts on the first day of its rule from the chosen one, and not in the past. */
  const first = $derived.by(() => {
    if (!day || !time) return null;
    return parsed ? nextTime(parsed, `${firstFrom(parsed, day)} ${time}`, localStamp()) : `${day} ${time}`;
  });
  const passed = $derived(!!first && !parsed && first <= localStamp());
  const ready = $derived(!!first && !unclear && !passed && (text !== null || !!about.trim()));

  /** Takes the keyboard from the editor behind; a new reminder starts at its text. */
  function focusOnOpen(el: HTMLElement) {
    (el.querySelector<HTMLElement>('input[data-first]') ?? el).focus();
  }
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onClose()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="overlay center" onclick={onClose}>
  <div
    class="modal reminder-dialog"
    role="dialog"
    tabindex="-1"
    aria-label="Напоминание"
    style="width: min(440px, 92vw);"
    use:focusOnOpen
    onclick={(e) => e.stopPropagation()}
  >
    <div class="modal-head">
      <h2>{at ? 'Напоминание' : 'Новое напоминание'}</h2>
      <button class="icon-btn" aria-label="Закрыть" onclick={onClose}><Icon name="x" /></button>
    </div>
    <form
      class="modal-body"
      onsubmit={(e) => {
        e.preventDefault();
        if (ready && first) onSave(first, parsed ? wanted : null, text ?? about.trim());
      }}
    >
      {#if text === null}
        <label class="field">О чём напомнить<input data-first bind:value={about} placeholder="Выпить витамины" /></label>
      {:else if text}
        <p class="reminder-about">{text}</p>
      {/if}
      <div class="reminder-when">
        <label class="field">День<input type="date" min={parsed ? undefined : localDate()} bind:value={day} /></label>
        <label class="field">Время<input type="time" bind:value={time} /></label>
      </div>
      <div class="field">
        Повторять
        <div class="reminder-repeats">
          {#each CHOICES as [label, value] (value)}
            <button
              type="button"
              class:active={wanted === value}
              aria-pressed={wanted === value}
              onclick={() => {
                chosen = value;
                custom = '';
              }}>{label}</button
            >
          {/each}
        </div>
        <input bind:value={custom} aria-label="Своё правило" placeholder="Своё: по вторникам и четвергам, 1-го числа" />
      </div>
      <p class="reminder-preview" class:bad={unclear || passed}>
        {#if unclear}
          Не понимаю такое правило. Можно так: каждые 2 недели, по вторникам и четвергам, каждый месяц 5-го.
        {:else if passed}
          Это время уже прошло.
        {:else if first && parsed}
          {at ? 'Следующий' : 'Первый'} раз {reminderLabel(first, localDate())}, дальше {wanted}.
        {:else if first}
          Напомнит {reminderLabel(first, localDate())}.
        {/if}
      </p>
      <div class="buttons">
        {#if onRemove}<button type="button" class="btn danger" onclick={onRemove}>Убрать</button>{/if}
        <button type="button" class="btn" onclick={onClose}>Отмена</button>
        <button type="submit" class="btn primary" disabled={!ready}>Сохранить</button>
      </div>
    </form>
  </div>
</div>
