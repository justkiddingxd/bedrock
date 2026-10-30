<script lang="ts" module>
  /** `renamingFolder` value that shows an empty row for a new top-level folder. */
  export const NEW_FOLDER = '__new__';
</script>

<script lang="ts">
  import Icon from './Icon.svelte';
  import { IS_MAC } from './WindowControls.svelte';
  import { displayName } from './api';
  import { slide } from 'svelte/transition';
  import { draggable, droppable, type Dnd } from './drag';
  import { ease, ms } from './motion';
  import type { Folder, View } from './views';

  interface Props {
    vaultName: string;
    vaultRoot: string;
    view: View;
    today: string;
    counts: { today: number; inbox: number; all: number; overdue: number };
    /** Subfolders and notes of a folder ('' = the vault root), sorted. */
    childrenOf: (dir: string) => { folders: Folder[]; notes: string[] };
    tags: { tag: string; count: number }[];
    pinned: string[];
    renamingFolder: string | null;
    syncText: string;
    syncState: string;
    dnd: Dnd;
    onNavigate: (v: View) => void;
    onFind: () => void;
    onVaultMenu: (e: MouseEvent) => void;
    onNewFolder: () => void;
    onFolderMenu: (e: MouseEvent, path: string) => void;
    onRenameFolder: (path: string, name: string | null) => void;
    onNoteMenu: (e: MouseEvent, path: string) => void;
    onHideTags: () => void;
    onSync: () => void;
    onSettings: (anchor: HTMLElement) => void;
    onHide: () => void;
  }
  let p: Props = $props();

  const TAG_LIMIT = 12;
  let allTags = $state(false);

  // Folders opened in the tree, remembered per vault.
  const expandedKey = $derived(`bedrock.expanded:${p.vaultRoot}`);
  let expanded = $state<string[]>([]);
  $effect(() => {
    try {
      expanded = JSON.parse(localStorage.getItem(expandedKey) ?? '[]');
    } catch {
      expanded = [];
    }
  });
  function setOpen(path: string, open: boolean) {
    if (expanded.includes(path) === open) return;
    expanded = open ? [...expanded, path] : expanded.filter((x) => x !== path);
    localStorage.setItem(expandedKey, JSON.stringify(expanded));
  }

  const current = $derived(p.view.kind === 'note' || p.view.kind === 'folder' ? p.view.path : null);
  /** A folder row is highlighted when it is open in the main area, or when it is collapsed and holds what is open. */
  const folderActive = (path: string) =>
    (p.view.kind === 'folder' && p.view.path === path) || (!expanded.includes(path) && !!current?.startsWith(`${path}/`));

  let gear = $state<HTMLButtonElement>();

  function autofocus(el: HTMLInputElement) {
    el.focus();
    el.select();
  }
  let renameDone = false;
  function finishRename(path: string, value: string | null) {
    if (renameDone) return;
    renameDone = true;
    p.onRenameFolder(path, value);
  }
  $effect(() => {
    if (p.renamingFolder) renameDone = false;
  });
  const contextMenu = (handler: (e: MouseEvent, path: string) => void, path: string) => (e: MouseEvent) => {
    e.preventDefault();
    handler(e, path);
  };
</script>

{#snippet renameRow(path: string, value: string)}
  <div class="nav-item small tree-item" style:--depth={0}>
    <Icon name="folder" size={15} />
    <input
      {value}
      placeholder="Название папки"
      aria-label="Название папки"
      use:autofocus
      onkeydown={(e) => {
        if (e.key === 'Enter') finishRename(path, e.currentTarget.value);
        if (e.key === 'Escape') finishRename(path, null);
      }}
      onblur={(e) => finishRename(path, e.currentTarget.value)}
    />
  </div>
{/snippet}

{#snippet branch(dir: string, depth: number)}
  {@const kids = p.childrenOf(dir)}
  {#each kids.folders as f (f.path)}
    {@const open = expanded.includes(f.path)}
    {#if p.renamingFolder === f.path}
      {@render renameRow(f.path, f.name)}
    {:else}
      <div class="nav-row" style:--depth={depth}>
        <button
          class="nav-item small tree-item"
          class:active={folderActive(f.path)}
          use:draggable={{ kind: 'folder', path: f.path }}
          use:droppable={p.dnd.intoFolder(f.path, () => setOpen(f.path, true))}
          onclick={() => {
            setOpen(f.path, true);
            p.onNavigate({ kind: 'folder', path: f.path });
          }}
          oncontextmenu={contextMenu(p.onFolderMenu, f.path)}
        >
          <Icon name="folder" size={15} /><span class="name">{f.name}</span>
          <span class="count">{f.count}</span>
        </button>
        <button
          class="tree-toggle"
          class:open
          aria-expanded={open}
          aria-label="{open ? 'Свернуть' : 'Развернуть'} папку {f.name}"
          title={open ? 'Свернуть' : 'Развернуть'}
          onclick={() => setOpen(f.path, !open)}
        >
          <Icon name="chevron-right" size={12} />
        </button>
        <button class="row-more" aria-label="Действия с папкой {f.name}" title="Действия" onclick={(e) => p.onFolderMenu(e, f.path)}>
          <Icon name="more" size={14} />
        </button>
      </div>
      {#if open}
        <div class="tree-children" transition:slide={{ duration: ms(170), easing: ease }}>{@render branch(f.path, depth + 1)}</div>
      {/if}
    {/if}
  {/each}
  <!-- Notes follow the folders; at the top level these are the notes outside any folder. -->
  {#each kids.notes as path (path)}
    <button
      class="nav-item small tree-item leaf"
      style:--depth={depth}
      class:active={p.view.kind === 'note' && p.view.path === path}
      use:draggable={{ kind: 'note', path }}
      use:droppable={p.dnd.intoFolder(dir)}
      onclick={() => p.onNavigate({ kind: 'note', path })}
      oncontextmenu={contextMenu(p.onNoteMenu, path)}
    >
      <Icon name="doc" size={14} /><span class="name">{displayName(path)}</span>
    </button>
  {/each}
  {#if depth > 0 && !kids.folders.length && !kids.notes.length}
    <div class="tree-empty" style:--depth={depth}>Пусто</div>
  {/if}
{/snippet}

<aside class="sidebar">
  <div class="vault-row" class:mac-pad={IS_MAC} data-tauri-drag-region>
    <button class="vault-btn" onclick={p.onVaultMenu} title="Хранилища">
      <span class="monogram" aria-hidden="true">{p.vaultName.charAt(0).toUpperCase()}</span>
      <span class="vault-name">{p.vaultName}</span>
      <Icon name="chevron-down" size={12} />
    </button>
    <span style="flex: 1; align-self: stretch;" data-tauri-drag-region></span>
    <button class="icon-btn" title="Скрыть боковую панель (Ctrl+\)" aria-label="Скрыть боковую панель" onclick={p.onHide}>
      <Icon name="sidebar" />
    </button>
  </div>

  <button class="find-btn" onclick={p.onFind}>
    <Icon name="search" size={14} />
    <span class="label">Найти или создать…</span>
    <span class="keys">Ctrl K</span>
  </button>

  <div class="sidebar-scroll">
    <nav class="nav" aria-label="Разделы">
      <button
        class="nav-item"
        class:active={p.view.kind === 'today'}
        title="Ctrl+Shift+D"
        use:droppable={p.dnd.onDay(p.today)}
        onclick={() => p.onNavigate({ kind: 'today', date: '' })}
      >
        <Icon name="sun" /><span class="name">Сегодня</span>
        {#if p.counts.today}<span class="count">{p.counts.today}</span>{/if}
      </button>
      <button class="nav-item" class:active={p.view.kind === 'upcoming'} onclick={() => p.onNavigate({ kind: 'upcoming' })}>
        <Icon name="calendar" /><span class="name">Предстоящие</span>
        {#if p.counts.overdue}<span class="count danger" title="Просрочено">{p.counts.overdue}</span>{/if}
      </button>
      <button class="nav-item" class:active={p.view.kind === 'all'} onclick={() => p.onNavigate({ kind: 'all' })}>
        <Icon name="notes" /><span class="name">Все заметки</span>
        <span class="count">{p.counts.all}</span>
      </button>
    </nav>

    <!-- Dropping on the heading or between rows takes a note or folder out of its folder. -->
    <div class="section-label" title="Перетащите сюда, чтобы вынести из папки" use:droppable={p.dnd.intoFolder('')}>
      <span>Папки</span>
      <button class="icon-btn" title="Новая папка" aria-label="Новая папка" onclick={p.onNewFolder}><Icon name="plus" size={13} /></button>
    </div>
    <nav class="nav tree" aria-label="Папки и заметки" use:droppable={p.dnd.intoFolder('')}>
      {#if p.renamingFolder === NEW_FOLDER}{@render renameRow(NEW_FOLDER, '')}{/if}
      {#if !p.childrenOf('').folders.length && p.renamingFolder !== NEW_FOLDER}
        <button class="nav-item small hint" onclick={p.onNewFolder}><Icon name="plus" size={15} /><span class="name">Создать папку</span></button>
      {/if}
      {@render branch('', 0)}
    </nav>

    <!-- Shown while a note is dragged even when nothing is pinned yet. -->
    <div class="pin-section" class:empty={!p.pinned.length}>
      <div class="section-label" use:droppable={p.dnd.pin(null)}><span>Закреплено</span></div>
      <nav class="nav" aria-label="Закреплённые заметки">
        {#each p.pinned as path (path)}
          <button
            class="nav-item small"
            class:active={p.view.kind === 'note' && p.view.path === path}
            use:draggable={{ kind: 'note', path }}
            use:droppable={p.dnd.pin(path)}
            onclick={() => p.onNavigate({ kind: 'note', path })}
            oncontextmenu={contextMenu(p.onNoteMenu, path)}
          >
            <Icon name="doc" size={15} /><span class="name">{displayName(path)}</span>
          </button>
        {/each}
        <div class="nav-item small hint pin-drop" use:droppable={p.dnd.pin(null)}>
          <Icon name="pin" size={15} /><span class="name">Перетащите сюда</span>
        </div>
      </nav>
    </div>

    {#if p.tags.length}
      <div class="tags-section">
        <div class="section-label">
          <span>Метки</span>
          <button class="icon-btn hide-btn" title="Скрыть метки" aria-label="Скрыть метки" onclick={p.onHideTags}><Icon name="x" size={12} /></button>
        </div>
        <nav class="nav" aria-label="Метки">
          {#each allTags ? p.tags : p.tags.slice(0, TAG_LIMIT) as t (t.tag)}
            <button
              class="nav-item small"
              class:active={p.view.kind === 'tag' && p.view.tag.toLowerCase() === t.tag.toLowerCase()}
              onclick={() => p.onNavigate({ kind: 'tag', tag: t.tag })}
            >
              <span class="tag-hash" aria-hidden="true">#</span><span class="name">{t.tag}</span>
              <span class="count">{t.count}</span>
            </button>
          {/each}
          {#if p.tags.length > TAG_LIMIT}
            <button class="nav-item small hint" onclick={() => (allTags = !allTags)}>
              <span class="name">{allTags ? 'Свернуть' : `Ещё ${p.tags.length - TAG_LIMIT}`}</span>
            </button>
          {/if}
        </nav>
      </div>
    {/if}
  </div>

  <div class="sidebar-foot">
    <button class="sync-btn" class:error={p.syncState === 'error'} onclick={p.onSync} title="Синхронизация">
      <Icon name="cloud" size={15} />
      <span>{p.syncText}</span>
    </button>
    <button class="icon-btn" bind:this={gear} title="Оформление (Ctrl+,)" aria-label="Оформление" onclick={() => gear && p.onSettings(gear)}>
      <Icon name="gear" size={15} />
    </button>
  </div>
</aside>
