<script lang="ts">
  import { onMount } from 'svelte';
  import { fly, slide } from 'svelte/transition';
  import { ease, ms } from './lib/motion';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { ask, open as openDialog } from '@tauri-apps/plugin-dialog';
  import { openPath, revealItemInDir } from '@tauri-apps/plugin-opener';
  import {
    CHANGED_EVENT,
    api,
    dailyDay,
    dailyPath,
    displayName,
    errText,
    insertTask,
    isNote,
    joinPath,
    parentDir,
    plural,
    safeName,
    type Entry,
    type NoteRef,
    type NoteInfo,
    type NoteSummary,
    type SyncStatus,
    type TaskItem,
    type VaultInfo,
  } from './lib/api';
  import { addDays, dayMonth, dueLabel, localDate, timeOf, weekdayShort } from './lib/dates';
  import { nextWeekday, parseQuickTask, taskLine } from './lib/quick';
  import { sameView, type Folder, type View } from './lib/views';
  import { droppable, type Dnd } from './lib/drag';
  import { loadPrefs, savePrefs } from './lib/prefs';
  import Sidebar, { NEW_FOLDER } from './lib/Sidebar.svelte';
  import FolderPicker from './lib/FolderPicker.svelte';
  import PromptDialog from './lib/PromptDialog.svelte';
  import TodayView from './lib/TodayView.svelte';
  import NoteView from './lib/NoteView.svelte';
  import NoteRail from './lib/NoteRail.svelte';
  import NoteList from './lib/NoteList.svelte';
  import TasksView from './lib/TasksView.svelte';
  import Switcher, { type CommandId } from './lib/Switcher.svelte';
  import SyncDialog from './lib/SyncDialog.svelte';
  import Integrations from './lib/Integrations.svelte';
  import Settings from './lib/Settings.svelte';
  import Welcome from './lib/Welcome.svelte';
  import WindowControls, { IS_MAC } from './lib/WindowControls.svelte';
  import Menu, { type MenuEntry } from './lib/Menu.svelte';
  import Icon from './lib/Icon.svelte';

  let vault = $state<VaultInfo | null>(null);
  let recent = $state<string[]>([]);
  let loading = $state(true);
  let files = $state<Entry[]>([]);
  let notes = $state<NoteRef[]>([]);
  let summaries = $state<NoteSummary[]>([]);
  let tasks = $state<TaskItem[]>([]);
  let revision = $state(0);
  let today = $state(localDate());

  let view = $state<View>({ kind: 'today', date: localDate() });
  let history = $state<View[]>([]);
  let historyPos = $state(-1);
  let mode = $state<'edit' | 'preview'>('edit');
  let focusTitle = $state(false);
  let showSidebar = $state(true);
  // The note's links and outline live in a panel on the right, hidden until asked for.
  const RAIL_KEY = 'bedrock.rail';
  let showRail = $state(localStorage.getItem(RAIL_KEY) === '1');
  let railInfo = $state<{ path: string; info: NoteInfo } | null>(null);
  function setRail(on: boolean) {
    showRail = on;
    localStorage.setItem(RAIL_KEY, on ? '1' : '0');
  }

  let switcherOpen = $state(false);
  let syncOpen = $state(false);
  let integrationsOpen = $state(false);
  let settingsAt = $state<{ x: number; y: number } | null>(null);
  let menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  let sync = $state<SyncStatus | null>(null);
  let pinned = $state<string[]>([]);
  let renamingFolder = $state<string | null>(null);
  type ToastAction = { label: string; run: () => void };
  let toast = $state<{ text: string; icon: string; action?: ToastAction } | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  let picker = $state<{ title: string; folders: string[]; current: string; pick: (folder: string) => void } | null>(null);
  let dialog = $state<{ title: string; label: string; value: string; action: string; submit: (v: string) => void } | null>(null);

  /** Untitled notes made by «Новая заметка» and not renamed yet; deleted if left without text. */
  const fresh = new Set<string>();

  let noteView = $state<ReturnType<typeof NoteView>>();
  let todayView = $state<ReturnType<typeof TodayView>>();

  const collator = new Intl.Collator('ru', { numeric: true, sensitivity: 'base' });

  function showToast(text: string, icon = 'check', action?: ToastAction) {
    toast = { text, icon, action };
    // Give the user time to reach an undo button.
    hideToastIn(action ? 10_000 : 4500);
  }
  function hideToastIn(ms: number) {
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), ms);
  }
  const fail = (e: unknown) => showToast(errText(e), 'warn');

  // ------------------------------------------------------------ derived data

  const counts = $derived.by(() => {
    const daily = dailyPath(today);
    return {
      today: tasks.filter((t) => !t.done && (t.due === today || (!t.due && t.path === daily))).length,
      overdue: tasks.filter((t) => !t.done && t.due && t.due < today).length,
      inbox: summaries.filter((s) => !s.path.includes('/')).length,
      all: summaries.length,
    };
  });

  function foldersIn(dir: string): Folder[] {
    const prefix = dir ? `${dir}/` : '';
    return files
      .filter((f) => f.isDir && f.path.startsWith(prefix) && !f.path.slice(prefix.length).includes('/'))
      .map((f) => ({
        path: f.path,
        name: f.path.slice(prefix.length),
        count: summaries.filter((s) => s.path.startsWith(`${f.path}/`)).length,
      }))
      .sort((a, b) => collator.compare(a.name, b.name));
  }
  function childrenOf(dir: string) {
    const notes = summaries
      .filter((s) => parentDir(s.path) === dir)
      .map((s) => s.path)
      .sort((a, b) => collator.compare(displayName(a), displayName(b)));
    return { folders: foldersIn(dir), notes };
  }
  const allFolders = $derived(files.filter((f) => f.isDir).map((f) => f.path).sort(collator.compare));

  /** Tags by number of notes; spelling of the first note that uses a tag wins. */
  const tagCounts = $derived.by(() => {
    const byKey = new Map<string, { tag: string; count: number }>();
    for (const s of summaries)
      for (const tag of new Set(s.tags)) {
        const key = tag.toLowerCase();
        const entry = byKey.get(key);
        if (entry) entry.count++;
        else byKey.set(key, { tag, count: 1 });
      }
    return [...byKey.values()].sort((a, b) => b.count - a.count || collator.compare(a.tag, b.tag));
  });

  const list = $derived.by(() => {
    const v = view;
    if (v.kind === 'all') return { title: 'Все заметки', notes: summaries, sub: [], showFolder: true, empty: 'В хранилище пока нет заметок.' };
    if (v.kind === 'inbox')
      return {
        title: 'Без папки',
        notes: summaries.filter((s) => !s.path.includes('/')),
        sub: [],
        showFolder: false,
        empty: 'Здесь появляются заметки, которые лежат вне папок.',
      };
    if (v.kind === 'folder')
      return {
        title: v.path.split('/').pop() ?? v.path,
        notes: summaries.filter((s) => parentDir(s.path) === v.path),
        sub: foldersIn(v.path),
        showFolder: false,
        empty: 'Папка пуста.',
      };
    if (v.kind === 'tag') {
      const tag = v.tag.toLowerCase();
      return {
        title: `#${v.tag}`,
        notes: summaries.filter((s) => s.tags.some((t) => t.toLowerCase() === tag || t.toLowerCase().startsWith(`${tag}/`))),
        sub: [],
        showFolder: true,
        empty: 'Заметок с этой меткой нет.',
      };
    }
    return null;
  });

  const crumbs = $derived.by(() => {
    if (view.kind !== 'note') return [];
    const parts = parentDir(view.path).split('/').filter(Boolean);
    return parts.map((name, i) => ({ name, path: parts.slice(0, i + 1).join('/') }));
  });

  function syncText(s: SyncStatus | null): string {
    if (!s || s.state === 'off') return 'Без синхронизации';
    if (s.state === 'syncing') return 'Синхронизация…';
    if (s.state === 'error') return s.needsLogin ? 'Нужно войти снова' : 'Нет связи с сервером';
    // Short enough for the sidebar; the button's tooltip says what it is about.
    return s.lastSync ? `Обновлено в ${timeOf(s.lastSync * 1000)}` : 'Синхронизировано';
  }

  // ------------------------------------------------------------ vault

  const pinnedKey = () => `bedrock.pinned:${vault?.root}`;

  async function openVault(path: string) {
    try {
      await flushAll();
      if (view.kind === 'note' && fresh.has(view.path)) await api.discardBlankNote(view.path);
      fresh.clear();
      const info = await api.openVault(path);
      // Load everything before the vault screens render, so no view starts empty.
      [files, notes, summaries, tasks, sync] = await Promise.all([
        api.listFiles(),
        api.allNotes(),
        api.listNotes(),
        api.listTasks(true),
        api.syncStatus(),
      ]);
      try {
        pinned = JSON.parse(localStorage.getItem(`bedrock.pinned:${info.root}`) ?? '[]');
      } catch {
        pinned = [];
      }
      history = [];
      historyPos = -1;
      view = { kind: 'today', date: today };
      vault = info;
      await navigate(view);
      recent = await api.recentVaults();
    } catch (e) {
      fail(e);
    }
  }

  async function pickVault() {
    const dir = await openDialog({ directory: true, multiple: false, title: 'Папка хранилища' });
    if (typeof dir === 'string') await openVault(dir);
  }

  async function refreshAll() {
    if (!vault) return;
    [files, notes, summaries, tasks] = await Promise.all([api.listFiles(), api.allNotes(), api.listNotes(), api.listTasks(true)]);
    pinned = pinned.filter((p) => summaries.some((s) => s.path === p));
    revision++;
  }

  function savePinned() {
    localStorage.setItem(pinnedKey(), JSON.stringify(pinned));
  }

  // ------------------------------------------------------------ navigation

  async function flushAll() {
    await Promise.all([noteView?.flush(), todayView?.flush()]);
  }

  async function navigate(v: View, how: 'push' | 'replace' | 'none' = 'push') {
    if (v.kind === 'today' && !v.date) v = { kind: 'today', date: today };
    if (v.kind === 'note' && !isNote(v.path)) {
      if (vault) openPath(`${vault.root}/${v.path}`).catch(fail);
      return;
    }
    await flushAll();
    const left = view;
    view = v;
    if (how === 'replace' && historyPos >= 0) history[historyPos] = v;
    else if (how === 'push' && !(historyPos >= 0 && sameView(history[historyPos], v))) {
      history = [...history.slice(0, historyPos + 1), v];
      historyPos = history.length - 1;
    }
    if (left.kind === 'note' && fresh.has(left.path) && !(v.kind === 'note' && v.path === left.path)) discardIfBlank(left.path);
  }

  /** Deletes an untitled note the user opened and left without writing anything. */
  async function discardIfBlank(path: string) {
    fresh.delete(path);
    try {
      if (!(await api.discardBlankNote(path))) return;
      dropHistory((h) => h.kind === 'note' && h.path === path);
      await refreshAll();
    } catch (e) {
      fail(e);
    }
  }

  function openNote(path: string, line?: number) {
    focusTitle = false;
    navigate({ kind: 'note', path, line });
  }

  async function goHistory(delta: number) {
    const pos = historyPos + delta;
    if (pos < 0 || pos >= history.length) return;
    historyPos = pos;
    focusTitle = false;
    await navigate(history[pos], 'none');
  }

  // ------------------------------------------------------------ notes & folders

  function contextFolder(): string {
    if (view.kind === 'folder') return view.path;
    if (view.kind === 'note' && !dailyDay(view.path)) return parentDir(view.path);
    return '';
  }

  async function newNote(folder = contextFolder()) {
    try {
      const path = await api.createNote(folder);
      fresh.add(path);
      await refreshAll();
      focusTitle = true;
      mode = 'edit';
      await navigate({ kind: 'note', path });
    } catch (e) {
      fail(e);
    }
  }

  async function createNamed(name: string) {
    switcherOpen = false;
    try {
      const path = await api.createNote(contextFolder(), safeName(name));
      await refreshAll();
      openNote(path);
    } catch (e) {
      fail(e);
    }
  }

  const itemName = (path: string) => (isNote(path) ? displayName(path) : (path.split('/').pop() ?? path));
  const isDirPath = (path: string) => files.some((f) => f.path === path && f.isDir);

  /** Top-level folders are named inline in the sidebar; nested ones in a dialog. Nothing is created until named. */
  function newFolder(parent = '') {
    if (!parent) {
      showSidebar = true;
      renamingFolder = NEW_FOLDER;
      return;
    }
    dialog = {
      title: 'Новая папка',
      label: `Название папки внутри «${itemName(parent)}»`,
      value: '',
      action: 'Создать',
      submit: async (name) => {
        dialog = null;
        try {
          await api.createFolder(joinPath(parent, safeName(name)));
          await refreshAll();
          navigate({ kind: 'folder', path: parent });
        } catch (e) {
          fail(e);
        }
      },
    };
  }

  async function renameFolder(path: string, name: string | null) {
    renamingFolder = null;
    const clean = name ? safeName(name) : '';
    if (!clean) return;
    if (path === NEW_FOLDER) {
      try {
        await api.createFolder(clean);
        await refreshAll();
      } catch (e) {
        fail(e);
      }
    } else if (clean !== itemName(path)) movePath(path, joinPath(parentDir(path), clean));
  }

  /** Top-level folders are renamed in place in the sidebar, nested ones in a dialog. */
  function startFolderRename(path: string) {
    if (!path.includes('/') && showSidebar) {
      renamingFolder = path;
      return;
    }
    dialog = {
      title: 'Переименовать папку',
      label: 'Название папки',
      value: itemName(path),
      action: 'Переименовать',
      submit: (name) => {
        dialog = null;
        renameFolder(path, name);
      },
    };
  }

  function renameNote(path: string) {
    dialog = {
      title: 'Переименовать',
      label: 'Название заметки',
      value: displayName(path),
      action: 'Переименовать',
      submit: (name) => {
        dialog = null;
        const clean = safeName(name);
        if (clean) movePath(path, joinPath(parentDir(path), `${clean}.md`));
      },
    };
  }

  function moveTo(path: string) {
    const isDir = isDirPath(path);
    picker = {
      title: `Переместить «${itemName(path)}» в…`,
      // A folder cannot go inside itself.
      folders: isDir ? allFolders.filter((f) => f !== path && !f.startsWith(`${path}/`)) : allFolders,
      current: parentDir(path),
      pick: (folder) => {
        picker = null;
        movePath(path, joinPath(folder, path.split('/').pop() ?? path), folder ? `Перемещено в «${folder}»` : 'Вынесено из папок');
      },
    };
  }

  /** Renames or moves a file or folder; links to it are rewritten by the backend. */
  async function movePath(from: string, to: string, done?: string) {
    if (from === to) return;
    try {
      await flushAll();
      const res = await api.renamePath(from, to);
      // A named note is kept even while empty.
      fresh.delete(from);
      const remap = (p: string) => (p === from ? res.path : p.startsWith(`${from}/`) ? res.path + p.slice(from.length) : p);
      const remapView = (v: View): View =>
        v.kind === 'note' ? { ...v, path: remap(v.path) } : v.kind === 'folder' ? { ...v, path: remap(v.path) } : v;
      history = history.map(remapView);
      view = remapView(view);
      pinned = pinned.map(remap);
      savePinned();
      await refreshAll();
      const n = res.updated.length;
      const links = n ? `ссылки обновлены в ${n} ${plural(n, ['заметке', 'заметках', 'заметках'])}` : '';
      const text = [done, links].filter(Boolean).join(' · ');
      if (text) showToast(text.charAt(0).toUpperCase() + text.slice(1), n ? 'link' : 'check');
    } catch (e) {
      fail(e);
    }
  }

  /** Drops history entries matching `gone` (and the repeats that leaves behind), keeping the position. */
  function dropHistory(gone: (v: View) => boolean) {
    const kept: View[] = [];
    let pos = -1;
    history.forEach((v, i) => {
      if (!gone(v) && !(kept.length && sameView(kept[kept.length - 1], v))) kept.push(v);
      if (i === historyPos) pos = kept.length - 1;
    });
    history = kept;
    historyPos = pos;
  }

  /** Moves to the vault trash right away and offers an undo; only a folder with notes asks first. */
  async function deletePath(path: string) {
    const isDir = isDirPath(path);
    const name = itemName(path);
    const inside = summaries.filter((s) => s.path.startsWith(`${path}/`)).length;
    if (isDir && inside) {
      const ok = await ask(
        `Папка «${name}» и ${inside} ${plural(inside, ['заметка', 'заметки', 'заметок'])} в ней будут перемещены в корзину хранилища.`,
        { title: 'Удалить папку?', kind: 'warning', okLabel: 'Удалить', cancelLabel: 'Отмена' },
      );
      if (!ok) return;
    }
    const gone = (v: View) => (v.kind === 'note' || v.kind === 'folder') && (v.path === path || v.path.startsWith(`${path}/`));
    const reopen = gone(view) ? view : null;
    const pins = pinned.filter((p) => p === path || p.startsWith(`${path}/`));
    // Going to the trash with an undo, so leaving the screen must not discard it.
    for (const f of fresh) if (f === path || f.startsWith(`${path}/`)) fresh.delete(f);
    try {
      await flushAll();
      // Leave the screen first so it does not report its own note as missing.
      if (reopen) {
        const back = history.slice(0, historyPos).filter((v) => !gone(v));
        await navigate(back[back.length - 1] ?? { kind: 'today', date: today });
      }
      const trashed = await api.deletePath(path);
      dropHistory(gone);
      pinned = pinned.filter((p) => !pins.includes(p));
      savePinned();
      await refreshAll();
      showToast(`«${name}» в корзине`, 'trash', {
        label: 'Отменить',
        run: () => restore(trashed, path, pins, reopen),
      });
    } catch (e) {
      fail(e);
    }
  }

  async function restore(trashed: string, path: string, pins: string[], reopen: View | null) {
    toast = null;
    try {
      const back = await api.restorePath(trashed, path);
      const remap = (p: string) => back + p.slice(path.length);
      pinned = [...pinned, ...pins.map(remap)];
      savePinned();
      await refreshAll();
      if (reopen?.kind === 'note' || reopen?.kind === 'folder') await navigate({ ...reopen, path: remap(reopen.path) });
      showToast(`«${itemName(back)}» снова на месте`);
    } catch (e) {
      fail(e);
    }
  }

  // ------------------------------------------------------------ drag and drop

  const dnd: Dnd = {
    intoFolder: (folder, onHold) => ({
      accepts: (d) =>
        (d.kind === 'note' || d.kind === 'folder') &&
        parentDir(d.path) !== folder &&
        !(d.kind === 'folder' && (folder === d.path || folder.startsWith(`${d.path}/`))),
      onDrop: (d) => {
        if (d.kind === 'note' || d.kind === 'folder')
          movePath(d.path, joinPath(folder, d.path.split('/').pop() ?? d.path), folder ? `Перемещено в «${folder}»` : 'Вынесено из папок');
      },
      onHold,
    }),
    onDay: (day) => ({
      accepts: (d) => d.kind === 'task' && !d.task.done && d.task.due !== day,
      onDrop: (d) => {
        if (d.kind === 'task') reschedule([d.task], day);
      },
    }),
    pin: (before) => ({
      accepts: (d) => d.kind === 'note' && d.path !== before && isNote(d.path),
      onDrop: (d) => {
        if (d.kind !== 'note') return;
        const rest = pinned.filter((p) => p !== d.path);
        const at = before ? rest.indexOf(before) : -1;
        rest.splice(at < 0 ? rest.length : at, 0, d.path);
        pinned = rest;
        savePinned();
      },
    }),
  };

  function setShowTags(on: boolean) {
    savePrefs({ ...loadPrefs(), showTags: on });
  }
  function hideTags() {
    setShowTags(false);
    showToast('Метки скрыты. Вернуть: шестерёнка внизу панели → Боковая панель', 'check', {
      label: 'Вернуть',
      run: () => {
        toast = null;
        setShowTags(true);
      },
    });
  }

  /** «Быстрая запись»: a new note outside folders, named after the first line. */
  async function capture(text: string) {
    const [first, ...rest] = text.split('\n');
    const name = safeName(first.replace(/^#+\s*/, '').slice(0, 60));
    const body = rest.join('\n').trim();
    try {
      const path = await api.createNote('', name || undefined);
      const before = await api.readNote(path).catch(() => '');
      await api.saveNote(path, `${body || text}\n`, before);
      await refreshAll();
      showToast(`Записано в заметку «${displayName(path)}»`, 'check', {
        label: 'Открыть',
        run: () => {
          toast = null;
          openNote(path);
        },
      });
    } catch (e) {
      fail(e);
      throw e;
    }
  }

  function togglePin(path: string) {
    pinned = pinned.includes(path) ? pinned.filter((p) => p !== path) : [...pinned, path];
    savePinned();
  }

  async function openDaily(date: string) {
    try {
      const path = await api.dailyNote(date);
      await refreshAll();
      openNote(path);
    } catch (e) {
      fail(e);
    }
  }

  // ------------------------------------------------------------ links & tags

  async function followLink(target: string, heading: string | null, from: string) {
    try {
      const path = target ? await api.followLink(target, from, true) : from;
      if (!path) return;
      if (!summaries.some((s) => s.path === path)) await refreshAll();
      let line: number | undefined;
      if (heading) {
        const info = await api.noteInfo(path);
        line = info?.headings.find((h) => h.text.toLowerCase() === heading.toLowerCase())?.line;
      }
      openNote(path, line);
    } catch (e) {
      fail(e);
    }
  }

  function followRaw(raw: string, from: string) {
    const [main] = raw.split('|');
    const [target, heading] = main.split('#');
    followLink(target.trim(), heading?.trim() || null, from);
  }

  function showTag(tag: string) {
    const clean = tag.replace(/^#/, '').trim();
    if (clean) navigate({ kind: 'tag', tag: clean });
  }

  // ------------------------------------------------------------ tasks

  async function toggleTask(t: TaskItem) {
    tasks = tasks.map((x) => (x === t ? { ...x, done: !x.done, doneDate: x.done ? null : today } : x));
    try {
      await api.toggleTask(t.path, t.line);
    } catch (e) {
      fail(e);
    }
    await refreshAll();
  }

  async function reschedule(list: TaskItem[], date: string | null) {
    if (!list.length) return;
    try {
      for (const t of list) await api.setTaskDue(t.path, t.line, date);
      const when = date ? dueLabel(date, today) : '';
      const n = list.length;
      if (n === 1) showToast(date ? `Срок: ${when}` : 'Срок убран', 'calendar');
      else if (date) showToast(`${n} ${plural(n, ['задача перенесена', 'задачи перенесены', 'задач перенесено'])} на ${when}`, 'calendar');
      else showToast(`Срок убран у ${n} ${plural(n, ['задачи', 'задач', 'задач'])}`, 'calendar');
    } catch (e) {
      fail(e);
    }
    await refreshAll();
    window.dispatchEvent(new CustomEvent(CHANGED_EVENT, { detail: list.map((t) => t.path) }));
  }

  /** Adds a task typed as plain text («отчёт в пятницу !») to the daily note of `date`. */
  async function addTask(raw: string, date: string) {
    const quick = parseQuickTask(raw, today);
    if (!quick.text) return;
    try {
      const path = dailyPath(date);
      const before = files.some((f) => f.path === path) ? await api.readNote(path) : '';
      await api.saveNote(path, insertTask(before, taskLine(quick)), before);
      await refreshAll();
      window.dispatchEvent(new CustomEvent(CHANGED_EVENT, { detail: [path] }));
      // A task due on another day leaves this list, so say where it went.
      const due = quick.due;
      if (due && due !== date)
        showToast(`Задача добавлена на ${dueLabel(due, today)}`, 'calendar', {
          label: 'Показать день',
          run: () => {
            toast = null;
            navigate({ kind: 'today', date: due });
          },
        });
    } catch (e) {
      fail(e);
    }
  }

  function taskMenu(e: MouseEvent, t: TaskItem) {
    const set = (date: string | null) => () => reschedule([t], date);
    const options: [string, string][] = [
      ['Сегодня', today],
      ['Завтра', addDays(today, 1)],
      ['Послезавтра', addDays(today, 2)],
    ];
    const monday = nextWeekday(today, 1);
    if (monday > addDays(today, 2)) options.push(['В понедельник', monday]);
    options.push(['Через неделю', addDays(today, 7)]);

    const items: MenuEntry[] = [{ heading: t.due ? `Срок: ${dueLabel(t.due, today)}` : 'Срок не задан' }];
    for (const [label, day] of options)
      if (day !== t.due)
        items.push({
          label,
          icon: day === today ? 'sun' : 'calendar',
          hint: `${weekdayShort(day).toLowerCase()}, ${dayMonth(day)}`,
          action: set(day),
        });
    if (t.due) items.push({ label: 'Убрать срок', icon: 'x', action: set(null) });
    items.push(
      '-',
      { label: t.done ? 'Вернуть в работу' : 'Выполнено', icon: 'check', action: () => toggleTask(t) },
      { label: 'Открыть в заметке', icon: 'doc', action: () => openNote(t.path, t.line) },
    );
    menu = { ...at(e), items };
  }

  // ------------------------------------------------------------ menus

  function at(e: MouseEvent) {
    const el = e.currentTarget as HTMLElement | null;
    if (e.type === 'contextmenu' || !el) return { x: e.clientX, y: e.clientY };
    const r = el.getBoundingClientRect();
    return { x: r.left, y: r.bottom + 4 };
  }

  function vaultMenu(e: MouseEvent) {
    const others = recent.filter((r) => r !== vault?.root);
    const items: MenuEntry[] = [];
    if (others.length) {
      items.push({ heading: 'Недавние хранилища' });
      for (const r of others) items.push({ label: r.split(/[\\/]/).pop() ?? r, icon: 'folder', action: () => openVault(r) });
      items.push('-');
    }
    items.push({ label: 'Открыть папку…', icon: 'plus', action: pickVault });
    if (vault) items.push({ label: 'Показать в проводнике', icon: 'reveal', action: () => revealItemInDir(vault!.root).catch(fail) });
    menu = { ...at(e), items };
  }

  function folderMenu(e: MouseEvent, path: string) {
    menu = {
      ...at(e),
      items: [
        { label: 'Новая заметка', icon: 'plus', action: () => newNote(path) },
        { label: 'Новая папка внутри', icon: 'folder', action: () => newFolder(path) },
        '-',
        { label: 'Переименовать', icon: 'pencil', action: () => startFolderRename(path) },
        { label: 'Переместить в…', icon: 'folder', action: () => moveTo(path) },
        { label: 'Показать в проводнике', icon: 'reveal', action: () => revealItemInDir(`${vault!.root}/${path}`).catch(fail) },
        '-',
        { label: 'Удалить', icon: 'trash', danger: true, action: () => deletePath(path) },
      ],
    };
  }

  function noteMenu(e: MouseEvent, path: string) {
    const items: MenuEntry[] = [];
    if (!(view.kind === 'note' && view.path === path)) items.push({ label: 'Открыть', icon: 'doc', action: () => openNote(path) });
    items.push(
      { label: pinned.includes(path) ? 'Открепить' : 'Закрепить в боковой панели', icon: 'pin', action: () => togglePin(path) },
      '-',
      { label: 'Переименовать', icon: 'pencil', action: () => renameNote(path) },
      { label: 'Переместить в…', icon: 'folder', action: () => moveTo(path) },
      { label: 'Показать в проводнике', icon: 'reveal', action: () => revealItemInDir(`${vault!.root}/${path}`).catch(fail) },
      '-',
      { label: 'Удалить', icon: 'trash', danger: true, action: () => deletePath(path) },
    );
    menu = { ...at(e), items };
  }

  function openSettings(anchor?: HTMLElement) {
    const r = anchor?.getBoundingClientRect();
    settingsAt = r ? { x: Math.max(8, r.left - 230), y: window.innerHeight - r.top + 8 } : { x: 16, y: 56 };
  }

  function command(id: CommandId) {
    switcherOpen = false;
    if (id === 'today') navigate({ kind: 'today', date: today });
    else if (id === 'upcoming') navigate({ kind: 'upcoming' });
    else if (id === 'inbox') navigate({ kind: 'inbox' });
    else if (id === 'all') navigate({ kind: 'all' });
    else if (id === 'new') newNote();
    else if (id === 'sync') syncOpen = true;
    else if (id === 'integrations') integrationsOpen = true;
    else if (id === 'settings') openSettings(document.querySelector<HTMLElement>('.sidebar-foot .icon-btn') ?? undefined);
    else if (id === 'vault') pickVault();
  }

  // ------------------------------------------------------------ events

  async function onVaultChanged(paths: string[]) {
    await refreshAll();
    window.dispatchEvent(new CustomEvent(CHANGED_EVENT, { detail: paths }));
    const v = view;
    if (v.kind === 'note' && !files.some((f) => f.path === v.path)) {
      showToast(`«${displayName(v.path)}» удалена или перемещена`, 'warn');
      navigate({ kind: 'today', date: today }, 'replace');
    }
  }

  let shownConflict = '';
  $effect(() => {
    const first = sync?.lastReport?.conflicts[0];
    if (first && first !== shownConflict) {
      shownConflict = first;
      showToast(`Конфликт синхронизации: сохранена копия «${first}»`, 'warn');
    }
  });

  function setMode(next: 'edit' | 'preview') {
    mode = next;
  }

  /** Mouse side buttons (3 = back, 4 = forward) move through the app's history, as in a browser. */
  function onMouseNav(e: MouseEvent) {
    if (e.button !== 3 && e.button !== 4) return;
    // Stop the WebView from navigating the page itself.
    e.preventDefault();
    e.stopPropagation();
    if (e.type !== 'mouseup' || !vault) return;
    menu = null;
    goHistory(e.button === 3 ? -1 : 1);
  }

  function onKeydown(e: KeyboardEvent) {
    if (!vault) return;
    const mod = e.ctrlKey || e.metaKey;
    const key = e.key.toLowerCase();
    let handled = true;
    if (mod && !e.shiftKey && ['k', 'o', 'p'].includes(key)) switcherOpen = true;
    else if (mod && !e.shiftKey && key === 'n') newNote();
    else if (mod && !e.shiftKey && key === 'e' && view.kind === 'note') setMode(mode === 'edit' ? 'preview' : 'edit');
    else if (mod && e.shiftKey && key === 'd') navigate({ kind: 'today', date: today });
    else if (mod && !e.shiftKey && key === 's') flushAll();
    // By key position, so the shortcuts work with any keyboard layout.
    else if (mod && !e.shiftKey && e.code === 'Backslash') showSidebar = !showSidebar;
    else if (mod && e.shiftKey && e.code === 'Backslash') setRail(!showRail);
    else if (mod && !e.shiftKey && key === ',') openSettings(document.querySelector<HTMLElement>('.sidebar-foot .icon-btn') ?? undefined);
    else if (e.altKey && e.key === 'ArrowLeft') goHistory(-1);
    else if (e.altKey && e.key === 'ArrowRight') goHistory(1);
    else handled = false;
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  }

  onMount(() => {
    const unlisten = [
      listen<string[]>('vault:changed', (e) => onVaultChanged(e.payload)),
      listen<SyncStatus>('sync:status', (e) => (sync = e.payload)),
      listen<{ path: string; line: number; text: string }>('reminder', (e) => {
        const r = e.payload;
        showToast(r.text, 'bell', {
          label: 'Открыть',
          run: () => {
            toast = null;
            openNote(r.path, r.line);
          },
        });
      }),
      // Closing the window also leaves the note: save it, and drop it if it is a blank new one.
      // Never let this hold the window open for long.
      getCurrentWindow().onCloseRequested(async () => {
        const leave = (async () => {
          await flushAll();
          if (view.kind === 'note' && fresh.has(view.path)) await api.discardBlankNote(view.path);
        })().catch(() => {});
        await Promise.race([leave, new Promise((r) => setTimeout(r, 1500))]);
      }),
    ];
    const clock = setInterval(() => (today = localDate()), 30_000);
    window.addEventListener('keydown', onKeydown, true);
    const mouseNav = ['mousedown', 'mouseup', 'auxclick'] as const;
    mouseNav.forEach((t) => window.addEventListener(t, onMouseNav, true));
    (async () => {
      recent = await api.recentVaults();
      if (recent.length) await openVault(recent[0]);
      loading = false;
    })();
    return () => {
      clearInterval(clock);
      window.removeEventListener('keydown', onKeydown, true);
      mouseNav.forEach((t) => window.removeEventListener(t, onMouseNav, true));
      unlisten.forEach((p) => p.then((f) => f()));
    };
  });
</script>

{#if !vault}
  {#if !loading}<Welcome {recent} onPick={pickVault} onOpen={openVault} />{/if}
{:else}
  <div class="app">
    {#if showSidebar}
      <div class="slide-wrap" transition:slide={{ axis: 'x', duration: ms(200), easing: ease }}>
      <Sidebar
        vaultName={vault.name}
        vaultRoot={vault.root}
        {view}
        {today}
        {counts}
        {childrenOf}
        tags={tagCounts}
        {pinned}
        {renamingFolder}
        syncText={syncText(sync)}
        syncState={sync?.state ?? 'off'}
        {dnd}
        onNavigate={(v) => (v.kind === 'note' ? openNote(v.path) : navigate(v))}
        onFind={() => (switcherOpen = true)}
        onVaultMenu={vaultMenu}
        onNewFolder={() => newFolder('')}
        onFolderMenu={folderMenu}
        onRenameFolder={renameFolder}
        onNoteMenu={noteMenu}
        onHideTags={hideTags}
        onSync={() => (syncOpen = true)}
        onSettings={openSettings}
        onHide={() => (showSidebar = false)}
      />
      </div>
    {/if}

    <main class="main">
      <header class="topbar" class:ruled={view.kind === 'note'} class:mac-pad={IS_MAC && !showSidebar} data-tauri-drag-region>
        {#if !showSidebar}
          <button class="icon-btn" title="Показать боковую панель (Ctrl+\)" aria-label="Показать боковую панель" onclick={() => (showSidebar = true)}>
            <Icon name="sidebar" />
          </button>
        {/if}
        <button class="icon-btn" aria-label="Назад" title="Назад (Alt+←)" disabled={historyPos <= 0} onclick={() => goHistory(-1)}>
          <Icon name="chevron-left" />
        </button>
        <button class="icon-btn" aria-label="Вперёд" title="Вперёд (Alt+→)" disabled={historyPos >= history.length - 1} onclick={() => goHistory(1)}>
          <Icon name="chevron-right" />
        </button>

        {#if view.kind === 'note'}
          {@const notePath = view.path}
          <div class="crumbs" data-tauri-drag-region>
            {#if dailyDay(notePath)}
              <button onclick={() => navigate({ kind: 'today', date: dailyDay(notePath)! })}>Дневник</button><span aria-hidden="true">›</span>
            {:else}
              {#each crumbs as c (c.path)}
                <button use:droppable={dnd.intoFolder(c.path)} onclick={() => navigate({ kind: 'folder', path: c.path })}>{c.name}</button
                ><span aria-hidden="true">›</span>
              {/each}
            {/if}
            <span class="current">{displayName(notePath)}</span>
          </div>
          <div class="segmented" role="group" aria-label="Режим">
            <button class:active={mode === 'edit'} aria-pressed={mode === 'edit'} onclick={() => setMode('edit')}>Правка</button>
            <button class:active={mode === 'preview'} aria-pressed={mode === 'preview'} title="Ctrl+E" onclick={() => setMode('preview')}>Чтение</button>
          </div>
          <button class="icon-btn" aria-label="Новая заметка" title="Новая заметка (Ctrl+N)" onclick={() => newNote()}><Icon name="plus" /></button>
          {@const backlinks = railInfo?.path === notePath ? railInfo.info.backlinks.length : 0}
          <button
            class="icon-btn rail-toggle"
            class:on={showRail}
            aria-pressed={showRail}
            aria-label="Панель «О заметке»"
            title="Ссылки, задачи и содержание (Ctrl+Shift+\)"
            onclick={() => setRail(!showRail)}
          >
            <Icon name="panel-right" />
            {#if !showRail && backlinks}<span class="rail-count" title="Ссылаются сюда">{backlinks}</span>{/if}
          </button>
          <button class="icon-btn" aria-label="Действия с заметкой" title="Действия" onclick={(e) => noteMenu(e, notePath)}><Icon name="more" /></button>
        {:else}
          <span class="spacer" data-tauri-drag-region></span>
          <button class="btn" title="Ctrl+N" onclick={() => newNote()}><Icon name="plus" size={13} />Новая заметка</button>
        {/if}
        <WindowControls />
      </header>

      <div class="workspace">
      <div class="view">
        {#if view.kind === 'today'}
          <TodayView
            bind:this={todayView}
            date={view.date}
            {today}
            {tasks}
            {files}
            {summaries}
            {pinned}
            getNotes={() => notes}
            onSelectDate={(d) => navigate({ kind: 'today', date: d }, 'replace')}
            onToggle={toggleTask}
            onOpen={openNote}
            onReschedule={reschedule}
            onTaskMenu={taskMenu}
            onNoteMenu={noteMenu}
            onMenu={(e, items) => (menu = { ...at(e), items })}
            onAddTask={addTask}
            onCapture={capture}
            {dnd}
            onOpenDaily={openDaily}
            onOpenLink={followRaw}
            onTag={showTag}
            onError={(m) => showToast(m, 'warn')}
          />
        {:else if view.kind === 'note'}
          {#key view.path}
            <NoteView
              bind:this={noteView}
              path={view.path}
              line={view.line}
              {mode}
              root={vault.root}
              autoFocusTitle={focusTitle}
              {revision}
              getNotes={() => notes}
              onFollow={(target, heading) => view.kind === 'note' && followLink(target, heading, view.path)}
              onTag={showTag}
              onOpenFolder={(path) => navigate(path ? { kind: 'folder', path } : { kind: 'inbox' })}
              onInfo={(info) => view.kind === 'note' && (railInfo = { path: view.path, info })}
              onOpenDay={(day) => navigate({ kind: 'today', date: day })}
              onRename={(name) => view.kind === 'note' && movePath(view.path, joinPath(parentDir(view.path), `${name}.md`))}
              onError={(m) => showToast(m, 'warn')}
            />
          {/key}
        {:else if view.kind === 'upcoming'}
          <TasksView {tasks} {today} {dnd} onToggle={toggleTask} onOpen={openNote} onReschedule={reschedule} onTaskMenu={taskMenu} onAddTask={addTask} />
        {:else if list}
          <NoteList
            title={list.title}
            notes={list.notes}
            subfolders={list.sub}
            showFolder={list.showFolder}
            emptyText={list.empty}
            dropInto={view.kind === 'folder' ? dnd.intoFolder(view.path) : view.kind === 'inbox' ? dnd.intoFolder('') : null}
            {dnd}
            onOpen={(p) => openNote(p)}
            onOpenFolder={(path) => navigate({ kind: 'folder', path })}
            onMenu={noteMenu}
            onFolderMenu={folderMenu}
          />
        {/if}
      </div>
      <!-- Stays open while moving between notes; the previous note's info shows until the new one is read. -->
      {#if showRail && view.kind === 'note'}
        <div class="slide-wrap" transition:slide={{ axis: 'x', duration: ms(200), easing: ease }}>
          {#if railInfo}
            {@const info = railInfo}
            <NoteRail
              path={info.path}
              info={info.info}
              onOpen={openNote}
              onFollow={(target, heading) => followLink(target, heading, info.path)}
              onGoToLine={(line) => noteView?.goToLine(line)}
              onClose={() => setRail(false)}
            />
          {/if}
        </div>
      {/if}
      </div>
    </main>
  </div>

  {#if switcherOpen}
    <Switcher
      onOpen={(p, line) => {
        switcherOpen = false;
        openNote(p, line);
      }}
      tags={tagCounts}
      onCreate={createNamed}
      onCommand={command}
      onTag={(tag) => {
        switcherOpen = false;
        showTag(tag);
      }}
      onClose={() => (switcherOpen = false)}
    />
  {/if}
  {#if syncOpen}
    <SyncDialog
      status={sync}
      vaultName={vault.name}
      onClose={() => (syncOpen = false)}
      onIntegrations={() => {
        syncOpen = false;
        integrationsOpen = true;
      }}
    />
  {/if}
  {#if integrationsOpen}
    <Integrations
      status={sync}
      onClose={() => (integrationsOpen = false)}
      onSync={() => {
        integrationsOpen = false;
        syncOpen = true;
      }}
    />
  {/if}
  {#if settingsAt}<Settings x={settingsAt.x} y={settingsAt.y} onClose={() => (settingsAt = null)} />{/if}
  {#if menu}<Menu x={menu.x} y={menu.y} items={menu.items} onClose={() => (menu = null)} />{/if}
  {#if picker}
    <FolderPicker title={picker.title} folders={picker.folders} current={picker.current} onPick={picker.pick} onClose={() => (picker = null)} />
  {/if}
  {#if dialog}
    <PromptDialog
      title={dialog.title}
      label={dialog.label}
      value={dialog.value}
      action={dialog.action}
      onSubmit={dialog.submit}
      onClose={() => (dialog = null)}
    />
  {/if}
{/if}

{#if toast}
  <!-- Hovering keeps the toast (and its undo button) on screen. -->
  <div
    class="toast"
    role="status"
    transition:fly={{ y: 14, duration: ms(200), easing: ease }}
    onmouseenter={() => clearTimeout(toastTimer)}
    onmouseleave={() => hideToastIn(3000)}
  >
    <Icon name={toast.icon} /><span>{toast.text}</span>
    {#if toast.action}<button class="link-btn" onclick={toast.action.run}>{toast.action.label}</button>{/if}
    <button class="icon-btn toast-close" aria-label="Скрыть" onclick={() => (toast = null)}><Icon name="x" size={13} /></button>
  </div>
{/if}
