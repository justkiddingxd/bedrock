/**
 * Fenced code blocks in the editor: drawn as a box, with «Копировать» (the
 * whole code, also when folded) and «Свернуть» down to the first lines.
 */

import { RangeSetBuilder, type EditorState, type Text } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin, WidgetType, type DecorationSet, type ViewUpdate } from '@codemirror/view';
import { codeFolding, foldEffect, foldedRanges, unfoldEffect } from '@codemirror/language';
import { plural } from '../api';
import { iconSvg } from '../Icon.svelte';
import { FOLD_KEEP, setFoldedBlocks } from '../codefold';

export interface CodeBlock {
  /** Number of the block in the note, from 0. */
  index: number;
  /** Line numbers of the fences; `close` is null while the block is not closed. */
  open: number;
  close: number | null;
  lines: number;
  codeFrom: number;
  codeTo: number;
}

const FENCE = /^\s*(`{3,}|~{3,})/;

export function findCodeBlocks(doc: Text): CodeBlock[] {
  const out: CodeBlock[] = [];
  const block = (open: number, close: number | null): CodeBlock => {
    const end = close ? close - 1 : doc.lines;
    const lines = Math.max(0, end - open);
    const codeFrom = lines ? doc.line(open + 1).from : doc.line(open).to;
    return { index: out.length, open, close, lines, codeFrom, codeTo: lines ? doc.line(end).to : codeFrom };
  };
  let open = 0;
  let fence = '';
  for (let n = 1; n <= doc.lines; n++) {
    const text = doc.line(n).text;
    const m = FENCE.exec(text);
    if (!open) {
      if (m) [open, fence] = [n, m[1]];
    } else if (m && m[1][0] === fence[0] && m[1].length >= fence.length && text.trim() === m[1]) {
      out.push(block(open, n));
      open = 0;
    }
  }
  if (open) out.push(block(open, null));
  return out;
}

/** What folding hides: everything after the first lines of code. */
function foldRange(doc: Text, b: CodeBlock) {
  return b.lines > FOLD_KEEP ? { from: doc.line(b.open + FOLD_KEEP).to, to: b.codeTo } : null;
}

function isFolded(state: EditorState, range: { from: number; to: number } | null): boolean {
  let found = false;
  if (range) foldedRanges(state).between(range.from, range.to, (from, to) => void (found ||= from === range.from && to === range.to));
  return found;
}

function blockAt(view: EditorView, index: number) {
  return findCodeBlocks(view.state.doc)[index];
}

export function copyBlock(view: EditorView, index: number): Promise<void> {
  const b = blockAt(view, index);
  return navigator.clipboard.writeText(b ? view.state.sliceDoc(b.codeFrom, b.codeTo) : '');
}

export function toggleFold(view: EditorView, index: number) {
  const b = blockAt(view, index);
  const range = b && foldRange(view.state.doc, b);
  if (!range) return;
  if (isFolded(view.state, range)) {
    view.dispatch({ effects: unfoldEffect.of(range) });
    return;
  }
  // The cursor must not stay in the hidden part.
  const inside = view.state.selection.ranges.some((r) => r.head > range.from && r.head <= range.to);
  view.dispatch({ effects: foldEffect.of(range), selection: inside ? { anchor: range.from } : undefined });
}

class Tools extends WidgetType {
  constructor(
    readonly index: number,
    readonly lines: number,
    readonly folded: boolean,
  ) {
    super();
  }

  eq(other: Tools) {
    return other.index === this.index && other.lines === this.lines && other.folded === this.folded;
  }

  toDOM(view: EditorView) {
    const box = document.createElement('span');
    box.className = 'cm-code-tools';
    const button = (label: string, title: string, run: (b: HTMLButtonElement) => void) => {
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'cm-code-btn';
      b.textContent = label;
      b.title = title;
      // Keep the cursor and focus where they are.
      b.onmousedown = (e) => e.preventDefault();
      b.onclick = (e) => {
        e.preventDefault();
        run(b);
      };
      box.append(b);
    };
    if (this.lines > FOLD_KEEP) {
      button(this.folded ? 'Развернуть' : 'Свернуть', this.folded ? 'Показать весь код' : `Оставить первые ${FOLD_KEEP} строки`, () =>
        toggleFold(view, this.index),
      );
    }
    button('', 'Скопировать весь код', (b) =>
      copyBlock(view, this.index).then(
        () => {
          b.innerHTML = iconSvg('check', 14);
          b.classList.add('done');
          b.title = 'Скопировано';
          setTimeout(() => {
            b.innerHTML = iconSvg('copy', 14);
            b.classList.remove('done');
            b.title = 'Скопировать весь код';
          }, 1500);
        },
        () => (b.title = 'Не удалось скопировать'),
      ),
    );
    const copy = box.lastElementChild as HTMLButtonElement;
    copy.classList.add('icon');
    copy.innerHTML = iconSvg('copy', 14);
    copy.setAttribute('aria-label', 'Скопировать код');
    return box;
  }

  ignoreEvent() {
    return true;
  }
}

export function codeBlocks(opts: { path: () => string }) {
  const plugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      reported = '';

      constructor(view: EditorView) {
        this.decorations = this.build(view);
      }

      update(u: ViewUpdate) {
        if (u.docChanged || foldedRanges(u.state) !== foldedRanges(u.startState)) this.decorations = this.build(u.view);
      }

      build(view: EditorView) {
        const { state } = view;
        const out = new RangeSetBuilder<Decoration>();
        const folded = new Set<number>();
        for (const b of findCodeBlocks(state.doc)) {
          const isClosed = b.close !== null;
          const last = b.close ?? state.doc.lines;
          const f = isFolded(state, foldRange(state.doc, b));
          if (f) folded.add(b.index);
          for (let n = b.open; n <= last; n++) {
            const line = state.doc.line(n);
            const cls = `cm-codeblock${n === b.open ? ' cm-codeblock-open' : ''}${isClosed && n === last ? ' cm-codeblock-close' : ''}`;
            out.add(line.from, line.from, Decoration.line({ class: cls }));
            if (n === b.open) out.add(line.to, line.to, Decoration.widget({ widget: new Tools(b.index, b.lines, f), side: 1 }));
          }
        }
        // Tell reading mode which blocks are folded here.
        const key = [...folded].join(',');
        if (key !== this.reported) {
          this.reported = key;
          setFoldedBlocks(opts.path(), folded);
        }
        return out.finish();
      }
    },
    { decorations: (v) => v.decorations },
  );

  return [
    codeFolding({
      preparePlaceholder: (state, range) => state.doc.lineAt(range.to).number - state.doc.lineAt(range.from).number,
      placeholderDOM: (_view, onclick, hidden: number) => {
        const el = document.createElement('span');
        el.className = 'cm-code-more';
        el.textContent = `··· ещё ${hidden} ${plural(hidden, ['строка', 'строки', 'строк'])}`;
        el.title = 'Развернуть';
        el.onclick = onclick;
        return el;
      },
    }),
    plugin,
  ];
}
