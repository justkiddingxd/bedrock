import { invoke } from '@tauri-apps/api/core';
import { dayMonth } from './dates';

export interface VaultInfo { root: string; name: string; notes: number }
export interface Entry { path: string; isDir: boolean; size: number; mtime: number }
export interface SaveResult { content: string; merged: boolean; conflict: string | null }
export interface Heading { level: number; text: string; line: number }
export interface Backlink { source: string; title: string; line: number; context: string }
export interface OutLink { target: string; heading: string | null; resolved: string | null; line: number; embed: boolean }
export interface Property { key: string; value: string }
export interface NoteInfo {
  path: string;
  title: string;
  aliases: string[];
  tags: string[];
  headings: Heading[];
  backlinks: Backlink[];
  outgoing: OutLink[];
  wordCount: number;
  mtime: number;
  properties: Property[];
  tasksTotal: number;
  tasksDone: number;
}
export interface NoteRef { path: string; title: string; linkText: string }
export interface NoteSummary { path: string; title: string; mtime: number; excerpt: string; tags: string[]; openTasks: number }
export interface Snippet { line: number; text: string }
export interface SearchHit { path: string; title: string; score: number; snippets: Snippet[] }
export interface SwitchHit { path: string; title: string; score: number }
export type Priority = 'high' | 'medium' | 'normal' | 'low';
export interface TaskItem {
  path: string;
  noteTitle: string;
  line: number;
  indent: number;
  status: string;
  done: boolean;
  text: string;
  due: string | null;
  scheduled: string | null;
  doneDate: string | null;
  /** "YYYY-MM-DD HH:MM" */
  reminder: string | null;
  /** How the task repeats, as written after 🔁. */
  recurrence: string | null;
  priority: Priority;
  tags: string[];
}
export interface SyncReport {
  pulled: number;
  pushed: number;
  deletedLocal: number;
  deletedRemote: number;
  merged: number;
  conflicts: string[];
  errors: string[];
  headSeq: number;
}
export interface SyncStatus {
  configured: boolean;
  server: string | null;
  username: string | null;
  vaultId: string | null;
  vaultName: string | null;
  mode: VaultMode | null;
  readOnly: boolean;
  shareReminders: boolean;
  state: 'off' | 'idle' | 'syncing' | 'error';
  lastSync: number | null;
  lastError: string | null;
  needsLogin: boolean;
  lastReport: SyncReport | null;
}
export type VaultMode = 'e2e' | 'server';
export type Role = 'owner' | 'editor' | 'viewer';
export interface ServerInfo {
  name: string;
  version: string;
  protocol: number;
  registration_open: boolean;
  max_file_mb: number;
  telegram_bot?: string | null;
}
export interface RemoteVault { id: string; name: string; created_at: number; head_seq: number; mode: VaultMode; role: Role; owner: string }
export interface LoginResult { username: string; server: ServerInfo; vaults: RemoteVault[] }
export interface Member { username: string; role: Role }
export interface TelegramStatus {
  available: boolean;
  bot: string | null;
  linked: boolean;
  notify_changes: boolean;
  own_bot: string | null;
  server_bot: string | null;
  linked_via: string | null;
}
export interface TelegramLink { url: string; code: string }
export interface ApiToken { id: number; name: string; created_at: number }
export interface ReminderWebhook { configured: boolean; url: string | null; last_status: string | null; last_delivery_at: number | null }
export interface WebhookTestResult { delivered: boolean; status: number | null; body: string; error: string | null }
export interface CreatedToken { id: number; name: string; token: string }
export interface McpInfo { binary: string | null; vault: string | null; server: string | null; serverVault: boolean }
export interface ReminderItem {
  path: string;
  noteTitle: string;
  line: number;
  at: string;
  text: string;
  done: boolean;
  task: boolean;
  repeat: string | null;
}

export const api = {
  recentVaults: () => invoke<string[]>('recent_vaults'),
  openVault: (path: string) => invoke<VaultInfo>('open_vault', { path }),
  listFiles: () => invoke<Entry[]>('list_files'),
  listNotes: () => invoke<NoteSummary[]>('list_notes'),
  readNote: (path: string) => invoke<string>('read_note', { path }),
  saveNote: (path: string, content: string, base: string) => invoke<SaveResult>('save_note', { path, content, base }),
  createNote: (folder?: string, name?: string) =>
    invoke<string>('create_note', { folder: folder ?? null, name: name ?? null }),
  createFolder: (path: string) => invoke<string>('create_folder', { path }),
  renamePath: (from: string, to: string) => invoke<{ path: string; updated: string[] }>('rename_path', { from, to }),
  deletePath: (path: string) => invoke<string>('delete_path', { path }),
  restorePath: (trashed: string, path: string) => invoke<string>('restore_path', { trashed, path }),
  discardBlankNote: (path: string) => invoke<boolean>('discard_blank_note', { path }),
  noteInfo: (path: string) => invoke<NoteInfo | null>('note_info', { path }),
  renderNote: (path: string, content: string) => invoke<string>('render_note', { path, content }),
  allNotes: () => invoke<NoteRef[]>('all_notes'),
  followLink: (target: string, from: string, create: boolean) =>
    invoke<string | null>('follow_link', { target, from, create }),
  search: (query: string) => invoke<SearchHit[]>('search', { query }),
  quickSwitch: (query: string) => invoke<SwitchHit[]>('quick_switch', { query }),
  listTasks: (includeDone: boolean) => invoke<TaskItem[]>('list_tasks', { includeDone }),
  toggleTask: (path: string, line: number) => invoke<string>('toggle_task', { path, line }),
  /** " " to do, "/" in progress, "x" done. */
  setTaskStatus: (path: string, line: number, status: string) => invoke<string>('set_task_status', { path, line, status }),
  /** A task line from the editor marked done or not, with the next occurrence of a repeating task. */
  taskLineDone: (line: string, done: boolean) => invoke<{ line: string; next: string | null }>('task_line_done', { line, done }),
  setTaskDue: (path: string, line: number, due: string | null) => invoke<string>('set_task_due', { path, line, due }),
  dailyNote: (date?: string) => invoke<string>('daily_note', { date: date ?? null }),

  syncStatus: () => invoke<SyncStatus>('sync_status'),
  syncLogin: (server: string, username: string, password: string, register: boolean) =>
    invoke<LoginResult>('sync_login', { server, username, password, register }),
  syncConnect: (vaultId: string | null, newName: string | null, mode: VaultMode | null, password: string | null) =>
    invoke<SyncStatus>('sync_connect', { vaultId, newName, mode, password }),
  syncNow: () => invoke<SyncReport>('sync_now'),
  syncDisconnect: () => invoke<SyncStatus>('sync_disconnect'),
  members: () => invoke<Member[]>('vault_members'),
  addMember: (username: string, role: 'editor' | 'viewer') => invoke<Member[]>('add_vault_member', { username, role }),
  removeMember: (username: string) => invoke<Member[]>('remove_vault_member', { username }),
  setShareReminders: (enabled: boolean) => invoke<void>('set_share_reminders', { enabled }),
  telegramStatus: () => invoke<TelegramStatus>('telegram_status'),
  telegramLink: () => invoke<TelegramLink>('telegram_link'),
  telegramSetChanges: (enabled: boolean) => invoke<TelegramStatus>('telegram_set_changes', { enabled }),
  telegramUnlink: () => invoke<TelegramStatus>('telegram_unlink'),
  telegramSetBot: (token: string) => invoke<TelegramStatus>('telegram_set_bot', { token }),
  telegramRemoveBot: () => invoke<TelegramStatus>('telegram_remove_bot'),
  reminderWebhook: () => invoke<ReminderWebhook>('reminder_webhook'),
  setReminderWebhook: (url: string, secret: string) => invoke<ReminderWebhook>('set_reminder_webhook', { url, secret }),
  deleteReminderWebhook: () => invoke<ReminderWebhook>('delete_reminder_webhook'),
  testReminderWebhook: () => invoke<WebhookTestResult>('test_reminder_webhook'),
  apiTokens: () => invoke<ApiToken[]>('api_tokens'),
  createApiToken: (name: string) => invoke<CreatedToken>('create_api_token', { name }),
  deleteApiToken: (id: number) => invoke<void>('delete_api_token', { id }),
  mcpInfo: () => invoke<McpInfo>('mcp_info'),
  listReminders: () => invoke<ReminderItem[]>('list_reminders'),
};

/** Window event fired with the paths that changed on disk; editors reload themselves. */
export const CHANGED_EVENT = 'bedrock:changed';

export const DAILY_DIR = 'Дневник';
export const dailyPath = (day: string) => `${DAILY_DIR}/${day}.md`;
const DAILY_RE = /^Дневник\/(\d{4}-\d{2}-\d{2})\.md$/;

/** The day of a daily note path, or null. */
export function dailyDay(path: string): string | null {
  return DAILY_RE.exec(path)?.[1] ?? null;
}

export const isNote = (path: string) => /\.(md|markdown)$/i.test(path);

/** File name without the note extension: "Folder/My note.md" -> "My note". */
export function noteName(path: string): string {
  const name = path.split('/').pop() ?? path;
  return isNote(name) ? name.replace(/\.(md|markdown)$/i, '') : name;
}

/** How a note is named in lists: daily notes by their date. */
export function displayName(path: string): string {
  const day = dailyDay(path);
  return day ? dayMonth(day) : noteName(path);
}

export function parentDir(path: string): string {
  const i = path.lastIndexOf('/');
  return i < 0 ? '' : path.slice(0, i);
}

export function joinPath(dir: string, name: string): string {
  return dir ? `${dir}/${name}` : name;
}

/** Russian plural: plural(3, ['заметка', 'заметки', 'заметок']) -> "заметки". */
export function plural(n: number, forms: [string, string, string]): string {
  const n10 = n % 10;
  const n100 = n % 100;
  if (n10 === 1 && n100 !== 11) return forms[0];
  if (n10 >= 2 && n10 <= 4 && (n100 < 12 || n100 > 14)) return forms[1];
  return forms[2];
}

/** A file or folder name without characters Windows forbids ("a/b?" -> "a-b"). */
export function safeName(name: string): string {
  return name
    .replace(/[\\/:*?"<>|]/g, '-')
    .replace(/[.\s]+$/, '')
    .trim();
}

/** One line of markdown as plain text: links by their visible text, no markers. */
export function plainText(s: string): string {
  return s
    .replace(/!?\[\[[^\]|]*\|([^\]]+)\]\]/g, '$1')
    .replace(/!?\[\[([^\]#]*)(?:#([^\]]*))?\]\]/g, (_, t: string, h: string | undefined) => t || h || '')
    .replace(/^\s*(?:>\s*)+/, '')
    .replace(/^\s*(?:[-*+]|\d+[.)])\s+(?:\[[ xX]\]\s+)?/, '')
    .replace(/^#{1,6}\s+/, '')
    .replace(/(?:📅|⏳|✅)\u{FE0F}?\s*\d{4}-\d{2}-\d{2}/gu, '')
    .replace(/[⏫🔼🔽🔺⏬]\u{FE0F}?/gu, '')
    .replace(/\*\*|__|`/g, '')
    .trim();
}

export function errText(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}

export function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
}

/** Appends "- [ ] task" to a note, under a "Задачи" heading if it has one. */
export function insertTask(content: string, task: string): string {
  const line = `- [ ] ${task}`;
  const lines = content.split('\n');
  const h = lines.findIndex((l) => /^#{1,6}\s+Задачи\s*$/i.test(l));
  if (h < 0) {
    const body = content.replace(/\s+$/, '');
    return body ? `${body}\n${line}\n` : `${line}\n`;
  }
  let i = h + 1;
  while (i < lines.length && (lines[i].trim() === '' || /^\s*[-*+] \[.\]/.test(lines[i]))) i++;
  let end = i;
  while (end > h + 1 && lines[end - 1].trim() === '') end--;
  lines.splice(end, 0, line);
  return lines.join('\n');
}
