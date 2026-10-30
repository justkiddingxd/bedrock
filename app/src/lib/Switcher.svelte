<script lang="ts" module>
  export const COMMANDS = [
    { id: 'today', label: 'Сегодня', icon: 'sun', keys: 'Ctrl Shift D' },
    { id: 'upcoming', label: 'Предстоящие задачи', icon: 'calendar' },
    { id: 'board', label: 'Доска задач', icon: 'board' },
    { id: 'inbox', label: 'Без папки', icon: 'inbox' },
    { id: 'all', label: 'Все заметки', icon: 'notes' },
    { id: 'new', label: 'Новая заметка', icon: 'plus', keys: 'Ctrl N' },
    { id: 'sync', label: 'Синхронизация и участники…', icon: 'cloud' },
    { id: 'integrations', label: 'Telegram и Claude…', icon: 'plug' },
    { id: 'settings', label: 'Оформление…', icon: 'gear', keys: 'Ctrl ,' },
    { id: 'vault', label: 'Открыть другое хранилище…', icon: 'folder' },
  ] as const;
  export type CommandId = (typeof COMMANDS)[number]['id'];
</script>

<script lang="ts">
  import Icon from './Icon.svelte';
  import { api, displayName, escapeHtml, parentDir, plainText, plural, type SearchHit, type SwitchHit } from './api';

  interface Props {
    tags: { tag: string; count: number }[];
    onOpen: (path: string, line?: number) => void;
    onCreate: (name: string) => void;
    onCommand: (id: CommandId) => void;
    onTag: (tag: string) => void;
    onClose: () => void;
  }
  let { tags, onOpen, onCreate, onCommand, onTag, onClose }: Props = $props();

  type Item =
    | { kind: 'tag'; tag: string; count: number }
    | { kind: 'note'; hit: SwitchHit }
    | { kind: 'text'; path: string; line: number; html: string }
    | { kind: 'cmd'; cmd: (typeof COMMANDS)[number] }
    | { kind: 'create'; name: string };
  interface Section {
    title: string;
    items: { item: Item; index: number }[];
  }

  let query = $state('');
  let names = $state<SwitchHit[]>([]);
  let texts = $state<SearchHit[]>([]);
  let selected = $state(0);
  let list = $state<HTMLDivElement>();

  $effect(() => {
    const q = query;
    const t = setTimeout(async () => {
      const [n, s] = await Promise.all([api.quickSwitch(q), q.trim() ? api.search(q) : Promise.resolve([])]);
      if (q !== query) return;
      names = n.slice(0, q.trim() ? 7 : 8);
      texts = s.slice(0, 5);
      selected = 0;
    }, 60);
    return () => clearTimeout(t);
  });

  function highlight(text: string, q: string) {
    let html = escapeHtml(text);
    const terms = q
      .toLowerCase()
      .split(/\s+/)
      .filter((w) => w && !w.startsWith('#'))
      .map((w) => escapeHtml(w).replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
    if (terms.length) html = html.replace(new RegExp(`(${terms.join('|')})`, 'gi'), '<mark>$1</mark>');
    return html;
  }

  const sections = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const out: Section[] = [];
    let index = 0;
    const add = (title: string, items: Item[]) => {
      if (items.length) out.push({ title, items: items.map((item) => ({ item, index: index++ })) });
    };
    // «#ра» lists matching tags first; picking one opens the tag's notes.
    if (q.startsWith('#')) {
      const t = q.slice(1);
      add(
        'Метки',
        tags
          .filter((x) => x.tag.toLowerCase().includes(t))
          .slice(0, 8)
          .map((x) => ({ kind: 'tag', tag: x.tag, count: x.count })),
      );
    }
    add(q ? 'Заметки' : 'Недавние', names.map((hit) => ({ kind: 'note', hit })));
    // A query naming a command («telegram», «синхр») runs it on Enter; creating
    // a note with that name stays one step down (and on Shift+Enter).
    const commands: Item[] = COMMANDS.filter((c) => !q || c.label.toLowerCase().includes(q)).map((cmd) => ({ kind: 'cmd', cmd }));
    const exact = names.some((h) => displayName(h.path).toLowerCase() === q);
    const create: Item[] = q && !q.startsWith('#') && !exact ? [{ kind: 'create', name: query.trim() }] : [];
    if (q && commands.length) {
      add('Команды', commands);
      add('', create);
    } else {
      add('', create);
      add('Команды', commands);
    }
    add(
      'В тексте',
      texts
        .filter((h) => h.snippets.length)
        .map((h) => ({ kind: 'text', path: h.path, line: h.snippets[0].line, html: highlight(plainText(h.snippets[0].text), query) })),
    );
    return out;
  });
  const flat = $derived(sections.flatMap((s) => s.items.map((i) => i.item)));

  function run(item: Item) {
    if (item.kind === 'tag') onTag(item.tag);
    else if (item.kind === 'note') onOpen(item.hit.path);
    else if (item.kind === 'text') onOpen(item.path, item.line);
    else if (item.kind === 'cmd') onCommand(item.cmd.id);
    else onCreate(item.name);
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') onClose();
    else if (e.key === 'ArrowDown') {
      e.preventDefault();
      selected = Math.min(selected + 1, flat.length - 1);
      scroll();
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      selected = Math.max(selected - 1, 0);
      scroll();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (e.shiftKey && query.trim()) onCreate(query.trim());
      else if (flat[selected]) run(flat[selected]);
    }
  }

  function scroll() {
    queueMicrotask(() => list?.querySelector('.selected')?.scrollIntoView({ block: 'nearest' }));
  }

  function autofocus(el: HTMLInputElement) {
    el.focus();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="overlay" onclick={onClose}>
  <div class="palette" role="dialog" tabindex="-1" aria-label="Найти или создать" onclick={(e) => e.stopPropagation()}>
    <label class="palette-input">
      <Icon name="search" size={16} />
      <input bind:value={query} {onkeydown} use:autofocus placeholder="Найти заметку, текст, #метку или команду" spellcheck="false" />
    </label>
    <div class="palette-list" bind:this={list}>
      {#each sections as s, si (si)}
        {#if s.title}<div class="palette-section">{s.title}</div>{/if}
        {#each s.items as { item, index } (index)}
          <button
            class="palette-item"
            class:selected={index === selected}
            onmousemove={() => (selected = index)}
            onclick={() => run(item)}
          >
            {#if item.kind === 'tag'}
              <span class="tag-hash" aria-hidden="true">#</span>
              <span class="texts"><span class="main-text">{item.tag}</span></span>
              <span class="side">{item.count} {plural(item.count, ['заметка', 'заметки', 'заметок'])}</span>
            {:else if item.kind === 'note'}
              <Icon name="doc" size={15} />
              <span class="texts">
                <span class="main-text">{displayName(item.hit.path)}</span>
                {#if item.hit.title !== displayName(item.hit.path)}<span class="sub-text">{item.hit.title}</span>{/if}
              </span>
              <span class="side">{parentDir(item.hit.path) || 'Без папки'}</span>
            {:else if item.kind === 'text'}
              <Icon name="search" size={15} />
              <span class="texts">
                <span class="main-text">{displayName(item.path)}</span>
                <span class="sub-text">{@html item.html}</span>
              </span>
            {:else if item.kind === 'cmd'}
              <Icon name={item.cmd.icon} size={15} />
              <span class="texts"><span class="main-text">{item.cmd.label}</span></span>
              {#if 'keys' in item.cmd}<span class="side">{item.cmd.keys}</span>{/if}
            {:else}
              <Icon name="plus" size={15} />
              <span class="texts"><span class="main-text">Создать заметку «{item.name}»</span></span>
              <span class="side">Shift Enter</span>
            {/if}
          </button>
        {/each}
      {/each}
    </div>
    <div class="palette-foot">
      <span>↑↓ выбрать</span><span>Enter открыть</span><span>Shift Enter создать</span><span>Esc закрыть</span>
    </div>
  </div>
</div>
