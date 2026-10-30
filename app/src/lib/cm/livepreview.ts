/**
 * Live preview, as in Obsidian: Markdown marks (#, **, _, ~~, `, [ ](…),
 * [[ ]], >, list bullets, ---) are hidden so the text reads formatted. The
 * lines under the cursor or selection show their marks for editing; when the
 * editor is not focused, every line is shown clean.
 */

import { syntaxTree } from '@codemirror/language';
import type { EditorState, Range } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin, WidgetType, type DecorationSet, type ViewUpdate } from '@codemirror/view';
import { openUrl } from '@tauri-apps/plugin-opener';

class Bullet extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const el = document.createElement('span');
    el.className = 'cm-lp-bullet';
    el.textContent = '•';
    return el;
  }
}

class Rule extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const el = document.createElement('span');
    el.className = 'cm-lp-hr';
    return el;
  }
}

const hide = Decoration.replace({});
const bullet = Decoration.replace({ widget: new Bullet() });
const rule = Decoration.replace({ widget: new Rule() });
const quoteLine = Decoration.line({ class: 'cm-lp-quote' });
const inlineCode = Decoration.mark({ class: 'cm-lp-code' });

const CODE = new Set(['FencedCode', 'CodeBlock', 'InlineCode']);
const WIKILINK = /\[\[([^[\]\n|]+)(\|[^[\]\n]+)?\]\]/g;

/** Lines whose marks stay visible: those touched by the selection, while focused. */
function activeLines(view: EditorView): Set<number> {
  const lines = new Set<number>();
  if (!view.hasFocus) return lines;
  for (const r of view.state.selection.ranges) {
    for (let n = view.state.doc.lineAt(r.from).number; n <= view.state.doc.lineAt(r.to).number; n++) lines.add(n);
  }
  return lines;
}

/** The last line of YAML frontmatter, which has its own display (0 when there is none). */
function frontmatterEnd(state: EditorState): number {
  const doc = state.doc;
  if (doc.lines < 2 || doc.line(1).text.trimEnd() !== '---') return 0;
  for (let n = 2; n <= doc.lines; n++) if (['---', '...'].includes(doc.line(n).text.trimEnd())) return n;
  return 0;
}

function inCode(state: EditorState, pos: number): boolean {
  for (let node: ReturnType<ReturnType<typeof syntaxTree>['resolveInner']> | null = syntaxTree(state).resolveInner(pos, 1); node; node = node.parent) {
    if (CODE.has(node.name)) return true;
  }
  return false;
}

function build(view: EditorView): DecorationSet {
  const { state } = view;
  const active = activeLines(view);
  const fmEnd = frontmatterEnd(state);
  const out: Range<Decoration>[] = [];
  const quoted = new Set<number>();
  const shown = (pos: number) => {
    const n = state.doc.lineAt(pos).number;
    return active.has(n) || n <= fmEnd;
  };
  /** A mark and the space after it. */
  const withSpace = (from: number, to: number) => (state.sliceDoc(to, to + 1) === ' ' ? to + 1 : to);

  for (const { from, to } of view.visibleRanges) {
    syntaxTree(state).iterate({
      from,
      to,
      enter: (node) => {
        if (node.name === 'FencedCode' || node.name === 'CodeBlock') return false;
        if (shown(node.from)) return;
        const parent = node.node.parent?.name ?? '';
        switch (node.name) {
          case 'HeaderMark':
            if (parent.startsWith('ATXHeading')) out.push(hide.range(node.from, withSpace(node.from, node.to)));
            break;
          case 'EmphasisMark':
          case 'StrikethroughMark':
            out.push(hide.range(node.from, node.to));
            break;
          case 'InlineCode':
            out.push(inlineCode.range(node.from, node.to));
            break;
          case 'CodeMark':
            if (parent === 'InlineCode') out.push(hide.range(node.from, node.to));
            break;
          case 'LinkMark':
            if (parent === 'Link' || parent === 'Autolink') out.push(hide.range(node.from, node.to));
            break;
          case 'URL':
          case 'LinkTitle':
            if (parent === 'Link') out.push(hide.range(node.from, node.to));
            break;
          case 'QuoteMark': {
            out.push(hide.range(node.from, withSpace(node.from, node.to)));
            const line = state.doc.lineAt(node.from);
            if (!quoted.has(line.number)) {
              quoted.add(line.number);
              out.push(quoteLine.range(line.from));
            }
            break;
          }
          case 'ListMark': {
            if (node.node.parent?.parent?.name !== 'BulletList') break;
            // A task shows only its checkbox; other items get a bullet.
            if (node.node.parent.getChild('Task')) out.push(hide.range(node.from, withSpace(node.from, node.to)));
            else out.push(bullet.range(node.from, node.to));
            break;
          }
          case 'HorizontalRule':
            out.push(rule.range(node.from, node.to));
            break;
        }
      },
    });

    // [[note|alias]]: only the name (or alias) is left.
    for (let n = state.doc.lineAt(from).number; n <= state.doc.lineAt(to).number; n++) {
      if (shown(state.doc.line(n).from)) continue;
      const line = state.doc.line(n);
      for (const m of line.text.matchAll(WIKILINK)) {
        const start = line.from + m.index;
        if (inCode(state, start)) continue;
        const end = start + m[0].length;
        out.push(hide.range(start, start + 2 + (m[2] ? m[1].length + 1 : 0)), hide.range(end - 2, end));
      }
    }
  }
  return Decoration.set(out, true);
}

/** Ctrl+click on a link opens it in the browser. */
function linkAt(view: EditorView, pos: number): string | null {
  for (let node: ReturnType<ReturnType<typeof syntaxTree>['resolveInner']> | null = syntaxTree(view.state).resolveInner(pos, 1); node; node = node.parent) {
    if (node.name === 'Link' || node.name === 'Autolink') {
      const url = node.getChild('URL');
      return url ? view.state.sliceDoc(url.from, url.to) : null;
    }
  }
  return null;
}

export const livePreview = [
  ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = build(view);
      }
      update(u: ViewUpdate) {
        if (u.docChanged || u.viewportChanged || u.selectionSet || u.focusChanged || syntaxTree(u.state) !== syntaxTree(u.startState)) {
          this.decorations = build(u.view);
        }
      }
    },
    { decorations: (v) => v.decorations },
  ),
  EditorView.domEventHandlers({
    mousedown(e, view) {
      if (!(e.ctrlKey || e.metaKey)) return false;
      const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
      const url = pos === null ? null : linkAt(view, pos);
      if (!url || !/^(https?:|mailto:)/i.test(url)) return false;
      e.preventDefault();
      openUrl(url).catch(() => {});
      return true;
    },
  }),
];
