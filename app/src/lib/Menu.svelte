<script lang="ts" module>
  export interface MenuItem {
    label: string;
    icon?: string;
    /** Secondary text on the right, e.g. a date. */
    hint?: string;
    danger?: boolean;
    action: () => void;
  }
  /** A compact button in a row: an icon or a short text, explained by its title. */
  export interface MenuButton {
    icon?: string;
    text?: string;
    title: string;
    active?: boolean;
    action: () => void;
  }
  /** An item, a separator ('-'), a small heading ({ heading }) or a row of buttons ({ row }). */
  export type MenuEntry = MenuItem | '-' | { heading: string } | { row: MenuButton[] };
</script>

<script lang="ts">
  import Icon from './Icon.svelte';

  let { x, y, items, onClose }: { x: number; y: number; items: MenuEntry[]; onClose: () => void } = $props();
  let el = $state<HTMLDivElement>();
  let left = $state(0);
  let top = $state(0);

  // Keep the menu inside the window.
  $effect(() => {
    const w = el?.offsetWidth ?? 220;
    const h = el?.offsetHeight ?? 200;
    left = Math.max(8, Math.min(x, window.innerWidth - w - 8));
    top = Math.max(8, Math.min(y, window.innerHeight - h - 8));
  });

  function pick(item: { action: () => void }) {
    onClose();
    item.action();
  }
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onClose()} onblur={onClose} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="menu-backdrop" onclick={onClose} oncontextmenu={(e) => { e.preventDefault(); onClose(); }}></div>
<div class="menu" bind:this={el} style:left="{left}px" style:top="{top}px" role="menu" tabindex="-1">
  {#each items as item, i (i)}
    {#if item === '-'}
      <hr />
    {:else if 'heading' in item}
      <div class="menu-label">{item.heading}</div>
    {:else if 'row' in item}
      <div class="menu-row" role="group">
        {#each item.row as b (b.title)}
          <button
            role={b.active === undefined ? 'menuitem' : 'menuitemcheckbox'}
            class="menu-chip"
            class:active={b.active}
            class:text={!b.icon}
            title={b.title}
            aria-label={b.title}
            aria-checked={b.active}
            onclick={() => pick(b)}
          >
            {#if b.icon}<Icon name={b.icon} size={15} />{:else}{b.text}{/if}
          </button>
        {/each}
      </div>
    {:else}
      <button role="menuitem" class:danger={item.danger} onclick={() => pick(item)}>
        {#if item.icon}<Icon name={item.icon} size={15} />{/if}
        <span>{item.label}</span>
        {#if item.hint}<span class="menu-hint">{item.hint}</span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 59;
  }
</style>
