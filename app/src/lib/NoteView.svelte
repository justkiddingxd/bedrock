<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import Editor from './Editor.svelte';
  import Preview from './Preview.svelte';
  import { api, dailyDay, displayName, errText, noteName, parentDir, safeName, type NoteInfo, type NoteRef } from './api';
  import { relTime, timeOf } from './dates';

  interface Props {
    path: string;
    line?: number;
    mode: 'edit' | 'preview';
    root: string;
    autoFocusTitle: boolean;
    revision: number;
    getNotes: () => NoteRef[];
    onFollow: (target: string, heading: string | null) => void;
    onTag: (tag: string) => void;
    onOpenFolder: (path: string) => void;
    /** Links, tasks and outline of the note, for the side panel. */
    onInfo: (info: NoteInfo) => void;
    onOpenDay: (day: string) => void;
    onRename: (name: string) => void;
    onError: (message: string) => void;
  }
  let p: Props = $props();

  let info = $state<NoteInfo | null>(null);
  let html = $state('');
  let ready = $state(false);
  let editor = $state<ReturnType<typeof Editor>>();
  let titleEl = $state<HTMLInputElement>();

  const day = $derived(dailyDay(p.path));
  const folder = $derived(parentDir(p.path));

  let source = $state<{ path: string; content: string } | null>(null);

  onMount(async () => {
    try {
      source = { path: p.path, content: await api.readNote(p.path) };
    } catch (e) {
      p.onError(errText(e));
    }
  });

  async function onLoaded() {
    ready = true;
    await tick();
    if (p.autoFocusTitle && !day) {
      titleEl?.focus();
      titleEl?.select();
    } else if (p.line === undefined && p.mode === 'edit') {
      editor?.focus();
    }
  }

  // Jump to a line (search hit, task, backlink), also when re-opened at another line.
  $effect(() => {
    const line = p.line;
    if (ready && line !== undefined && p.mode === 'edit') untrack(() => editor?.goToLine(line));
  });

  $effect(() => {
    void p.revision;
    api.noteInfo(p.path).then((i) => {
      info = i;
      if (i) p.onInfo(i);
    });
  });

  $effect(() => {
    if (p.mode !== 'preview' || !ready) return;
    void p.revision;
    untrack(async () => {
      await editor?.flush();
      html = await api.renderNote(p.path, editor?.getText() ?? '');
    });
  });

  export function flush() {
    return editor?.flush();
  }

  export function goToLine(line: number) {
    editor?.goToLine(line);
  }

  function followRaw(raw: string) {
    const [main] = raw.split('|');
    const [target, heading] = main.split('#');
    p.onFollow(target.trim(), heading?.trim() || null);
  }

  async function toggleInPreview(line: number) {
    try {
      await editor?.flush();
      const content = await api.toggleTask(p.path, line);
      editor?.replaceContent(content);
      html = await api.renderNote(p.path, content);
    } catch (e) {
      p.onError(errText(e));
    }
  }
</script>

<div class="note-wrap">
  <article class="note-article">
    {#if day}
      <h1 class="note-title">{displayName(p.path)}</h1>
    {:else}
      <input
        class="note-title"
        bind:this={titleEl}
        value={noteName(p.path)}
        placeholder="Без названия"
        spellcheck="false"
        aria-label="Название заметки"
        onchange={(e) => {
          const typed = e.currentTarget.value.trim();
          const v = safeName(typed);
          if (!v) {
            e.currentTarget.value = noteName(p.path);
            return;
          }
          // File names cannot hold \ / : * ? " < > |, so say what was replaced.
          if (v !== typed) {
            e.currentTarget.value = v;
            p.onError('Символы \\ / : * ? " < > | нельзя использовать в названии, заменены на «-»');
          }
          if (v !== noteName(p.path)) p.onRename(v);
        }}
        onkeydown={(e) => {
          if (e.key === 'Enter' || e.key === 'Escape') {
            if (e.key === 'Escape') e.currentTarget.value = noteName(p.path);
            e.preventDefault();
            editor?.focus(true);
          }
        }}
      />
    {/if}

    <dl class="props">
      {#if day}
        <dt>День</dt>
        <dd><button onclick={() => p.onOpenDay(day)}>Открыть на главной</button></dd>
      {:else}
        <dt>Папка</dt>
        <dd><button onclick={() => p.onOpenFolder(folder)}>{folder || 'Без папки'}</button></dd>
      {/if}
      {#if info}
        <dt>Изменено</dt>
        <dd>{relTime(info.mtime)}{relTime(info.mtime).includes(':') ? '' : `, ${timeOf(info.mtime)}`}</dd>
        {#each info.properties as prop (prop.key)}
          <dt>{prop.key}</dt>
          <dd>{prop.value}</dd>
        {/each}
        {#if info.tags.length}
          <dt>Метки</dt>
          <dd>
            {#each info.tags as tag (tag)}
              <button class="tag-chip" onclick={() => p.onTag(`#${tag}`)}>#{tag}</button>
            {/each}
          </dd>
        {/if}
      {/if}
    </dl>
    <div class="props-rule"></div>

    <div style:display={p.mode === 'preview' ? 'none' : 'block'}>
      <Editor
        bind:this={editor}
        {source}
        onLoad={onLoaded}
        getNotes={p.getNotes}
        onOpenLink={followRaw}
        onTag={p.onTag}
        onError={p.onError}
        placeholderText="Начните писать. [[ — ссылка, Ctrl+L — задача, @завтра — срок"
      />
    </div>
    {#if p.mode === 'preview'}
      <Preview {html} path={p.path} root={p.root} onOpenLink={p.onFollow} onTag={p.onTag} onToggleTask={toggleInPreview} />
    {/if}
  </article>

</div>
