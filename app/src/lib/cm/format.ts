/** Markdown formatting commands for the editor: inline marks, line blocks, inserts, task markers, clipboard. */

import { EditorSelection, type ChangeSpec, type EditorState } from '@codemirror/state';
import type { EditorView } from '@codemirror/view';
import { startCompletion } from '@codemirror/autocomplete';

function done(view: EditorView) {
  view.focus();
  return true;
}

/** The text a mark applies to: the selection, or the word under an empty cursor. */
function target(state: EditorState, range: { from: number; to: number }) {
  if (range.from !== range.to) return { from: range.from, to: range.to };
  return state.wordAt(range.from) ?? { from: range.from, to: range.from };
}

/** Whether the main selection (or the word under the cursor) is wrapped in `mark`. */
export function isWrapped(state: EditorState, mark: string): boolean {
  const { from, to } = target(state, state.selection.main);
  const n = mark.length;
  if (state.sliceDoc(from - n, from) === mark && state.sliceDoc(to, to + n) === mark) return true;
  const text = state.sliceDoc(from, to);
  return text.length >= 2 * n && text.startsWith(mark) && text.endsWith(mark);
}

/** Wraps the selection in `mark` (**, _, ~~, `), or unwraps it when it already is. */
export function toggleWrap(view: EditorView, mark: string): boolean {
  const { state } = view;
  const n = mark.length;
  const tr = state.changeByRange((range) => {
    const { from, to } = target(state, range);
    if (state.sliceDoc(from - n, from) === mark && state.sliceDoc(to, to + n) === mark) {
      return {
        changes: [
          { from: from - n, to: from },
          { from: to, to: to + n },
        ],
        range: EditorSelection.range(from - n, to - n),
      };
    }
    const text = state.sliceDoc(from, to);
    if (text.length >= 2 * n && text.startsWith(mark) && text.endsWith(mark)) {
      return { changes: { from, to, insert: text.slice(n, -n) }, range: EditorSelection.range(from, to - 2 * n) };
    }
    return {
      changes: [
        { from, insert: mark },
        { from: to, insert: mark },
      ],
      range: from === to ? EditorSelection.cursor(from + n) : EditorSelection.range(from + n, to + n),
    };
  });
  view.dispatch(state.update(tr, { scrollIntoView: true, userEvent: 'input.format' }));
  return done(view);
}

// ------------------------------------------------------------ line blocks

export type Block = 'bullet' | 'number' | 'task' | 'quote';

const LINE = /^(\s*)(#{1,6}\s+)?(>\s?)?([-*+]\s+\[[ xX]\]\s+|[-*+]\s+|\d+[.)]\s+)?/;

/** What the line starts with: heading level and list or quote kind. */
export function lineKind(text: string): { heading: number; block: Block | null; done: boolean } {
  const m = LINE.exec(text)!;
  const list = m[4] ?? '';
  const block: Block | null = /\[[ xX]\]/.test(list) ? 'task' : /^[-*+]/.test(list) ? 'bullet' : list ? 'number' : m[3] ? 'quote' : null;
  return { heading: m[2] ? m[2].trim().length : 0, block, done: /\[[xX]\]/.test(list) };
}

/** Distinct lines touched by the selection, in order. */
function selectedLines(state: EditorState) {
  const seen = new Set<number>();
  const lines = [];
  for (const r of state.selection.ranges) {
    for (let n = state.doc.lineAt(r.from).number; n <= state.doc.lineAt(r.to).number; n++) {
      if (!seen.has(n)) {
        seen.add(n);
        lines.push(state.doc.line(n));
      }
    }
  }
  return lines;
}

function rewriteLines(view: EditorView, rewrite: (text: string, index: number) => string) {
  const changes: ChangeSpec[] = selectedLines(view.state).map((line, i) => ({ from: line.from, to: line.to, insert: rewrite(line.text, i) }));
  view.dispatch({ changes, userEvent: 'input.format' });
  return done(view);
}

/** Heading of the given level for the selected lines; the same level again (or 0) makes plain text. */
export function setHeading(view: EditorView, level: number): boolean {
  const current = lineKind(view.state.doc.lineAt(view.state.selection.main.head).text).heading;
  const next = current === level ? 0 : level;
  return rewriteLines(view, (text) => {
    const m = LINE.exec(text)!;
    const rest = text.slice(m[1].length + (m[2]?.length ?? 0));
    return m[1] + (next ? `${'#'.repeat(next)} ` : '') + rest;
  });
}

/** Turns the selected lines into a list, numbered list, task list or quote; the same kind again removes it. */
export function setBlock(view: EditorView, block: Block): boolean {
  const off = lineKind(view.state.doc.lineAt(view.state.selection.main.head).text).block === block;
  return rewriteLines(view, (text, i) => {
    const m = LINE.exec(text)!;
    const rest = text.slice(m[0].length);
    const indent = m[1];
    if (off) return indent + rest;
    const prefix = { bullet: '- ', number: `${i + 1}. `, task: '- [ ] ', quote: '> ' }[block];
    return indent + prefix + rest;
  });
}

/** Marks a task line done or not done. */
export function setDone(view: EditorView, on: boolean): boolean {
  return rewriteLines(view, (text) => text.replace(/^(\s*[-*+]\s+\[)[ xX](\])/, `$1${on ? 'x' : ' '}$2`));
}

// ------------------------------------------------------------ inserts

/** Replaces the selection with `before + selection + after` and selects or places the cursor as asked. */
function surround(view: EditorView, before: string, after: string, select: 'inside' | 'after' = 'inside'): boolean {
  view.dispatch(
    view.state.changeByRange((range) => {
      const text = view.state.sliceDoc(range.from, range.to);
      const insert = before + text + after;
      const at = select === 'inside' ? range.from + before.length : range.from + insert.length;
      return {
        changes: { from: range.from, to: range.to, insert },
        range: select === 'inside' ? EditorSelection.range(at, at + text.length) : EditorSelection.cursor(at),
      };
    }),
    { userEvent: 'input.format', scrollIntoView: true },
  );
  return done(view);
}

/** Markdown link: a selected address becomes the target, other text becomes the title. */
export function insertLink(view: EditorView): boolean {
  const { from, to } = view.state.selection.main;
  const text = view.state.sliceDoc(from, to);
  if (/^https?:\/\/\S+$/.test(text)) {
    view.dispatch({ changes: { from, to, insert: `[](${text})` }, selection: { anchor: from + 1 }, userEvent: 'input.format' });
    return done(view);
  }
  view.dispatch({
    changes: { from, to, insert: `[${text}]()` },
    selection: { anchor: from + text.length + (text ? 3 : 1) },
    userEvent: 'input.format',
  });
  return done(view);
}

/** Link to a note: wraps the selection in [[ ]], or opens note suggestions. */
export function insertWikilink(view: EditorView): boolean {
  const empty = view.state.selection.main.empty;
  surround(view, '[[', ']]');
  if (empty) startCompletion(view);
  return true;
}

export function insertCodeBlock(view: EditorView): boolean {
  const { state } = view;
  const first = state.doc.lineAt(state.selection.main.from);
  const last = state.doc.lineAt(state.selection.main.to);
  if (state.selection.main.empty && !first.text.trim()) {
    view.dispatch({ changes: { from: first.from, to: first.to, insert: '```\n\n```' }, selection: { anchor: first.from + 4 }, userEvent: 'input.format' });
    return done(view);
  }
  view.dispatch({
    changes: [
      { from: first.from, insert: '```\n' },
      { from: last.to, insert: '\n```' },
    ],
    userEvent: 'input.format',
  });
  return done(view);
}

/** Inserts a block (table, divider) on its own line below the cursor's line. */
export function insertBlock(view: EditorView, block: string): boolean {
  const line = view.state.doc.lineAt(view.state.selection.main.head);
  const lead = line.text.trim() ? '\n\n' : '';
  const at = line.text.trim() ? line.to : line.from;
  view.dispatch({
    changes: { from: at, to: line.text.trim() ? at : line.to, insert: `${lead}${block}\n` },
    selection: { anchor: at + lead.length + block.length + 1 },
    userEvent: 'input.format',
    scrollIntoView: true,
  });
  return done(view);
}

export const TABLE = '| Столбец | Столбец |\n| --- | --- |\n|  |  |';

// ------------------------------------------------------------ task markers

const DUE = /\s*📅\s*\d{4}-\d{2}-\d{2}/u;
const REMIND = /\s*⏰\s*\d{4}-\d{2}-\d{2}\s+\d{1,2}:\d{2}/u;

/** Sets the due date (📅) or reminder (⏰) of the selected lines, replacing one already there. */
export function setMarker(view: EditorView, kind: 'due' | 'remind', value: string): boolean {
  const re = kind === 'due' ? DUE : REMIND;
  const mark = kind === 'due' ? '📅' : '⏰';
  return rewriteLines(view, (text) => `${text.replace(re, '').trimEnd()} ${mark} ${value}`);
}

// ------------------------------------------------------------ clipboard

function selectedText(view: EditorView): string {
  return view.state.selection.ranges.map((r) => view.state.sliceDoc(r.from, r.to)).join('\n');
}

export async function copy(view: EditorView, cut: boolean): Promise<void> {
  const text = selectedText(view);
  if (!text) return;
  await navigator.clipboard.writeText(text);
  if (cut) view.dispatch(view.state.replaceSelection(''), { userEvent: 'delete.cut' });
  view.focus();
}

export async function paste(view: EditorView): Promise<void> {
  const text = await navigator.clipboard.readText();
  view.dispatch(view.state.replaceSelection(text), { userEvent: 'input.paste', scrollIntoView: true });
  view.focus();
}

export function selectAll(view: EditorView): boolean {
  view.dispatch({ selection: { anchor: 0, head: view.state.doc.length } });
  return done(view);
}
