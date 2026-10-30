<script lang="ts">
  import { untrack } from 'svelte';
  import Icon from './Icon.svelte';

  interface Props {
    title: string;
    label: string;
    value?: string;
    action: string;
    onSubmit: (value: string) => void;
    onClose: () => void;
  }
  let { title, label, value = '', action, onSubmit, onClose }: Props = $props();
  // The dialog edits its own copy; `value` is only the starting text.
  let text = $state(untrack(() => value));

  function autofocus(el: HTMLInputElement) {
    el.focus();
    el.select();
  }
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onClose()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="overlay center" onclick={onClose}>
  <div class="modal" role="dialog" tabindex="-1" aria-label={title} style="width: min(400px, 92vw);" onclick={(e) => e.stopPropagation()}>
    <div class="modal-head">
      <h2>{title}</h2>
      <button class="icon-btn" aria-label="Закрыть" onclick={onClose}><Icon name="x" /></button>
    </div>
    <form
      class="modal-body"
      onsubmit={(e) => {
        e.preventDefault();
        if (text.trim()) onSubmit(text.trim());
      }}
    >
      <label class="field">{label}<input bind:value={text} use:autofocus spellcheck="false" /></label>
      <div class="buttons">
        <button type="button" class="btn" onclick={onClose}>Отмена</button>
        <button type="submit" class="btn primary" disabled={!text.trim()}>{action}</button>
      </div>
    </form>
  </div>
</div>
