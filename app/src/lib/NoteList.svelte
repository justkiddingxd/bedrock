<script lang="ts">
  import Icon from './Icon.svelte';
  import { displayName, parentDir, plural, type NoteSummary } from './api';
  import { relTime } from './dates';
  import { draggable, droppable, type Dnd, type DropSpec } from './drag';
  import type { Folder } from './views';

  interface Props {
    title: string;
    notes: NoteSummary[];
    subfolders?: Folder[];
    showFolder?: boolean;
    emptyText: string;
    /** Notes dropped on the list itself go here (the folder it shows). */
    dropInto?: DropSpec | null;
    dnd: Dnd;
    onOpen: (path: string) => void;
    onOpenFolder: (path: string) => void;
    onMenu: (e: MouseEvent, path: string) => void;
    onFolderMenu?: (e: MouseEvent, path: string) => void;
  }
  let { title, notes, subfolders = [], showFolder = true, emptyText, dropInto = null, dnd, onOpen, onOpenFolder, onMenu, onFolderMenu }: Props =
    $props();

  const SORT_KEY = 'bedrock.sort';
  let sort = $state<'mtime' | 'name'>(localStorage.getItem(SORT_KEY) === 'name' ? 'name' : 'mtime');
  const collator = new Intl.Collator('ru', { numeric: true, sensitivity: 'base' });
  const sorted = $derived(
    sort === 'name' ? [...notes].sort((a, b) => collator.compare(displayName(a.path), displayName(b.path))) : notes,
  );

  function setSort(s: 'mtime' | 'name') {
    sort = s;
    localStorage.setItem(SORT_KEY, s);
  }
  const menu = (handler: (e: MouseEvent, path: string) => void, path: string) => (e: MouseEvent) => {
    e.preventDefault();
    handler(e, path);
  };
</script>

<div class="list-view" use:droppable={dropInto}>
  <div class="list-head">
    <h1 class="list-title">{title}<span class="list-count">{notes.length} {plural(notes.length, ['заметка', 'заметки', 'заметок'])}</span></h1>
    {#if notes.length > 1}
      <div class="segmented" role="group" aria-label="Сортировка">
        <button class:active={sort === 'mtime'} aria-pressed={sort === 'mtime'} onclick={() => setSort('mtime')}>Недавние</button>
        <button class:active={sort === 'name'} aria-pressed={sort === 'name'} onclick={() => setSort('name')}>По названию</button>
      </div>
    {/if}
  </div>

  {#each subfolders as f (f.path)}
    <button
      class="row"
      class:no-folder={!showFolder}
      use:draggable={{ kind: 'folder', path: f.path }}
      use:droppable={dnd.intoFolder(f.path)}
      onclick={() => onOpenFolder(f.path)}
      oncontextmenu={onFolderMenu ? menu(onFolderMenu, f.path) : undefined}
    >
      <span class="rt"><Icon name="folder" size={15} />{f.name}</span>
      {#if showFolder}<span class="rf"></span>{/if}
      <span class="rd">{f.count}</span>
    </button>
  {/each}

  {#each sorted as n (n.path)}
    <button
      class="row"
      class:no-folder={!showFolder}
      use:draggable={{ kind: 'note', path: n.path }}
      onclick={() => onOpen(n.path)}
      oncontextmenu={menu(onMenu, n.path)}
    >
      <span style="min-width: 0;">
        <span class="rt">{displayName(n.path)}</span>
        {#if n.excerpt}<span class="re" style="display: block;">{n.excerpt}</span>{/if}
      </span>
      {#if showFolder}<span class="rf">{parentDir(n.path) || 'Без папки'}</span>{/if}
      <span class="rd">{relTime(n.mtime)}</span>
    </button>
  {/each}

  {#if notes.length === 0 && subfolders.length === 0}
    <div class="empty">{emptyText}</div>
  {/if}
</div>
