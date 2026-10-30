<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte';
  import { Annotation, Compartment, EditorState, type Extension } from '@codemirror/state';
  import { EditorView, drawSelection, dropCursor, highlightSpecialChars, keymap, placeholder } from '@codemirror/view';
  import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
  import { highlightSelectionMatches, searchKeymap } from '@codemirror/search';
  import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap } from '@codemirror/autocomplete';
  import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
  import { languages } from '@codemirror/language-data';
  import { bracketMatching, indentOnInput, syntaxHighlighting } from '@codemirror/language';
  import { CHANGED_EVENT, api, errText, type NoteRef, type SaveResult } from './api';
  import Menu, { type MenuEntry } from './Menu.svelte';
  import { toggleWrap } from './cm/format';
  import { editorMenu } from './cm/menu';
  import { codeBlocks } from './cm/codeblocks';
  import { livePreview } from './cm/livepreview';
  import { PREFS_EVENT, loadPrefs, type Prefs } from './prefs';
  import { bedrockHighlight, bedrockTheme } from './cm/theme';
  import { dateCompletion, toggleTask, wikilinkCompletion, wikilinks } from './cm/wikilinks';
  import { decorations } from './cm/decor';
  import { reminderPhrases } from './cm/reminders';

  interface Props {
    getNotes: () => NoteRef[];
    onOpenLink: (raw: string) => void;
    onTag: (tag: string) => void;
    onSaved?: (path: string, result: SaveResult) => void;
    onError: (message: string) => void;
    placeholderText?: string;
    /** Document to show; a new object loads it (an alternative to calling load()). */
    source?: { path: string; content: string } | null;
    /** Called after a document has been loaded into the view. */
    onLoad?: () => void;
  }
  let { getNotes, onOpenLink, onTag, onSaved, onError, placeholderText = '', source = null, onLoad }: Props = $props();

  const SAVE_DELAY = 400;
  /** Marks programmatic edits (loading, merges) that must not trigger a save. */
  const External = Annotation.define<boolean>();

  let host: HTMLDivElement;
  let view: EditorView | undefined;
  let extensions: Extension[] = [];
  let path = '';
  /** Content last loaded from or saved to disk; the merge base for saves. */
  let base = '';
  let timer: ReturnType<typeof setTimeout> | undefined;
  let queue: Promise<void> = Promise.resolve();
  let menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  /** Live preview, switched in the appearance settings. */
  const preview = new Compartment();
  const previewOf = (on: boolean) => (on ? livePreview : []);

  function onPrefs(e: Event) {
    const on = (e as CustomEvent<Prefs>).detail.livePreview;
    // Documents loaded later use the new setting too.
    extensions = buildExtensions();
    view?.dispatch({ effects: preview.reconfigure(previewOf(on)) });
  }

  /** Right click: our menu with formatting. Shift + right click keeps the system one (spelling suggestions). */
  function contextMenu(e: MouseEvent, v: EditorView): boolean {
    if (e.shiftKey) return false;
    e.preventDefault();
    // Like the system menu: a click outside the selection moves the cursor there.
    const pos = v.posAtCoords({ x: e.clientX, y: e.clientY });
    if (pos !== null && !v.state.selection.ranges.some((r) => pos >= r.from && pos <= r.to)) v.dispatch({ selection: { anchor: pos } });
    menu = { x: e.clientX, y: e.clientY, items: editorMenu(v, pos, { onOpenLink: (raw) => onOpenLink(raw), onError: (m) => onError(m) }) };
    return true;
  }

  function buildExtensions(): Extension[] {
    return [
      highlightSpecialChars(),
      history(),
      drawSelection(),
      dropCursor(),
      indentOnInput(),
      bracketMatching(),
      closeBrackets(),
      highlightSelectionMatches(),
      EditorView.lineWrapping,
      EditorView.contentAttributes.of({ spellcheck: 'true', autocorrect: 'off' }),
      placeholder(placeholderText),
      markdown({ base: markdownLanguage, codeLanguages: languages }),
      syntaxHighlighting(bedrockHighlight),
      bedrockTheme,
      autocompletion({ override: [wikilinkCompletion(() => getNotes()), dateCompletion()], icons: false }),
      wikilinks({ onOpenLink: (raw) => onOpenLink(raw), onTag: (tag) => onTag(tag) }),
      decorations,
      reminderPhrases,
      codeBlocks({ path: () => path }),
      preview.of(previewOf(loadPrefs().livePreview)),
      EditorView.domEventHandlers({ contextmenu: contextMenu }),
      keymap.of([
        { key: 'Mod-l', run: toggleTask },
        { key: 'Mod-b', run: (v) => toggleWrap(v, '**') },
        { key: 'Mod-i', run: (v) => toggleWrap(v, '_') },
        ...closeBracketsKeymap,
        ...defaultKeymap,
        ...searchKeymap,
        ...historyKeymap,
        ...completionKeymap,
        indentWithTab,
      ]),
      EditorView.updateListener.of((u) => {
        if (u.docChanged && !u.transactions.some((tr) => tr.annotation(External))) schedule();
      }),
    ];
  }

  function schedule() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = undefined;
      enqueueSave();
    }, SAVE_DELAY);
  }

  /** Saves run strictly one after another so each one sees the previous base. */
  function enqueueSave(): Promise<void> {
    queue = queue.then(save, save);
    return queue;
  }

  async function save() {
    if (!view || !path) return;
    const p = path;
    const content = view.state.doc.toString();
    if (content === base) return;
    try {
      const res = await api.saveNote(p, content, base);
      if (p !== path) return;
      if (res.content === content) {
        base = content;
      } else if (view.state.doc.toString() === content) {
        // Merged with a change made on disk (sync, another app): show it.
        base = res.content;
        setDoc(res.content);
      } else {
        // The user kept typing during the merge: merge again on the next save.
        base = content;
        schedule();
      }
      onSaved?.(p, res);
    } catch (e) {
      onError(`Не удалось сохранить: ${errText(e)}`);
    }
  }

  function setDoc(text: string) {
    if (!view) return;
    const sel = view.state.selection.main;
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: text },
      selection: { anchor: Math.min(sel.anchor, text.length), head: Math.min(sel.head, text.length) },
      annotations: External.of(true),
    });
  }

  export async function load(p: string, content: string) {
    await flush();
    path = p;
    base = content;
    // Called before onMount (parent effects can run first): applied on mount.
    if (view) view.setState(EditorState.create({ doc: content, extensions }));
    onLoad?.();
  }

  /** Writes pending edits to disk now. */
  export async function flush() {
    if (timer) {
      clearTimeout(timer);
      timer = undefined;
      enqueueSave();
    }
    await queue;
  }

  export function isDirty(): boolean {
    return timer !== undefined || (view !== undefined && view.state.doc.toString() !== base);
  }

  export function getText(): string {
    return view?.state.doc.toString() ?? '';
  }

  /** Replaces the content after a change made elsewhere in the app. */
  export function replaceContent(content: string) {
    base = content;
    setDoc(content);
  }

  export function goToLine(line: number) {
    if (!view) return;
    const l = view.state.doc.line(Math.max(1, Math.min(line + 1, view.state.doc.lines)));
    view.dispatch({ selection: { anchor: l.from }, effects: EditorView.scrollIntoView(l.from, { y: 'center' }) });
    view.focus();
  }

  export function focus(atEnd = false) {
    if (!view) return;
    if (atEnd) view.dispatch({ selection: { anchor: view.state.doc.length } });
    view.focus();
  }

  /** Reloads from disk when the file changed elsewhere and nothing is pending here. */
  async function onChanged(e: Event) {
    const paths = (e as CustomEvent<string[]>).detail;
    if (!path || !paths.includes(path) || isDirty()) return;
    try {
      const content = await api.readNote(path);
      if (content !== base && !isDirty()) replaceContent(content);
    } catch {
      // Deleted or moved: the app navigates away.
    }
  }

  onMount(() => {
    extensions = buildExtensions();
    view = new EditorView({ parent: host, state: EditorState.create({ doc: base, extensions }) });
    window.addEventListener(CHANGED_EVENT, onChanged);
    window.addEventListener(PREFS_EVENT, onPrefs);
  });

  // Declarative loading (after onMount, so the view exists).
  $effect(() => {
    const s = source;
    if (s) untrack(() => load(s.path, s.content));
  });

  onDestroy(() => {
    window.removeEventListener(CHANGED_EVENT, onChanged);
    window.removeEventListener(PREFS_EVENT, onPrefs);
    // Leaving the view: save what is pending without waiting for the timer.
    if (view && path && (timer || view.state.doc.toString() !== base)) {
      clearTimeout(timer);
      const content = view.state.doc.toString();
      const p = path;
      const b = base;
      queue.then(() => api.saveNote(p, content, b)).catch((e) => onError(`Не удалось сохранить: ${errText(e)}`));
    }
    view?.destroy();
  });
</script>

<div class="editor" bind:this={host}></div>
{#if menu}
  <Menu
    x={menu.x}
    y={menu.y}
    items={menu.items}
    onClose={() => {
      menu = null;
      view?.focus();
    }}
  />
{/if}
