<script lang="ts">
  import Icon from './Icon.svelte';
  import { displayName, plainText, plural, type NoteInfo } from './api';

  interface Props {
    path: string;
    info: NoteInfo;
    onOpen: (path: string, line?: number) => void;
    onFollow: (target: string, heading: string | null) => void;
    onGoToLine: (line: number) => void;
    onClose: () => void;
  }
  let p: Props = $props();

  const outgoing = $derived.by(() => {
    const seen = new Set<string>();
    return p.info.outgoing.filter((l) => {
      const key = l.resolved ?? `?${l.target.toLowerCase()}`;
      if (!l.target || seen.has(key) || l.resolved === p.path) return false;
      seen.add(key);
      return true;
    });
  });
  const topLevel = $derived(Math.min(...p.info.headings.map((h) => h.level)));
</script>

<aside class="note-panel" aria-label="О заметке">
  <div class="note-panel-head">
    <span>О заметке</span>
    <button class="icon-btn" title="Скрыть панель (Ctrl+Shift+\)" aria-label="Скрыть панель" onclick={p.onClose}><Icon name="x" size={14} /></button>
  </div>

  <div class="rail-label"><span>Ссылаются сюда</span><span>{p.info.backlinks.length}</span></div>
  {#each p.info.backlinks as b (`${b.source}:${b.line}`)}
    <button class="backlink-card" onclick={() => p.onOpen(b.source, b.line)}>
      <span class="bt">{displayName(b.source)}</span>
      <span class="bc">{plainText(b.context)}</span>
    </button>
  {:else}
    <div class="rail-none">Пока ни одна заметка не ссылается сюда</div>
  {/each}

  {#if outgoing.length}
    <div class="rail-label"><span>Ссылки</span><span>{outgoing.length}</span></div>
    {#each outgoing as l (`${l.target}:${l.line}`)}
      <button
        class="rail-link"
        class:missing={!l.resolved}
        title={l.resolved ? l.resolved : 'Такой заметки нет — нажмите, чтобы создать'}
        onclick={() => (l.resolved ? p.onOpen(l.resolved) : p.onFollow(l.target, null))}
      >
        <Icon name={l.resolved ? 'doc' : 'plus'} size={14} /><span>{l.resolved ? displayName(l.resolved) : l.target}</span>
      </button>
    {/each}
  {/if}

  {#if p.info.tasksTotal}
    <div class="rail-label"><span>Задачи</span><span>{p.info.tasksDone} из {p.info.tasksTotal}</span></div>
    <div class="progress" aria-hidden="true"><div style:width="{(p.info.tasksDone / p.info.tasksTotal) * 100}%"></div></div>
  {/if}

  {#if p.info.headings.length > 1}
    <div class="rail-label"><span>Содержание</span></div>
    {#each p.info.headings as h (h.line)}
      <button class="outline-link" style:padding-left="{(h.level - topLevel) * 12}px" onclick={() => p.onGoToLine(h.line)}>{h.text}</button>
    {/each}
  {/if}

  <div class="rail-foot">
    {p.info.wordCount} {plural(p.info.wordCount, ['слово', 'слова', 'слов'])}<br />{p.path}
  </div>
</aside>
