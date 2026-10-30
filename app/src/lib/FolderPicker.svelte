<script lang="ts">
  import Icon from './Icon.svelte';

  interface Props {
    title: string;
    folders: string[];
    /** Folder the item is in now ('' = vault root); shown but not offered. */
    current: string;
    onPick: (folder: string) => void;
    onClose: () => void;
  }
  let { title, folders, current, onPick, onClose }: Props = $props();

  let query = $state('');
  let selected = $state(0);
  let list = $state<HTMLDivElement>();

  const options = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const all = ['', ...folders].filter((f) => f !== current);
    return all.filter((f) => !q || (f || 'без папки').toLowerCase().includes(q));
  });

  $effect(() => {
    void query;
    selected = 0;
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') onClose();
    else if (e.key === 'ArrowDown') {
      e.preventDefault();
      selected = Math.min(selected + 1, options.length - 1);
      queueMicrotask(() => list?.querySelector('.selected')?.scrollIntoView({ block: 'nearest' }));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      selected = Math.max(selected - 1, 0);
      queueMicrotask(() => list?.querySelector('.selected')?.scrollIntoView({ block: 'nearest' }));
    } else if (e.key === 'Enter' && options[selected] !== undefined) {
      e.preventDefault();
      onPick(options[selected]);
    }
  }

  function autofocus(el: HTMLInputElement) {
    el.focus();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="overlay" onclick={onClose}>
  <div class="palette" role="dialog" tabindex="-1" aria-label={title} onclick={(e) => e.stopPropagation()}>
    <label class="palette-input">
      <Icon name="folder" size={16} />
      <input bind:value={query} {onkeydown} use:autofocus placeholder={title} spellcheck="false" />
    </label>
    <div class="palette-list" bind:this={list}>
      {#each options as f, i (f)}
        <button class="palette-item" class:selected={i === selected} onmousemove={() => (selected = i)} onclick={() => onPick(f)}>
          <Icon name={f ? 'folder' : 'inbox'} size={15} />
          <span class="texts"><span class="main-text">{f || 'Без папки'}</span></span>
        </button>
      {:else}
        <div class="empty">Папок с таким названием нет</div>
      {/each}
    </div>
    <div class="palette-foot"><span>↑↓ выбрать</span><span>Enter переместить</span><span>Esc закрыть</span></div>
  </div>
</div>
