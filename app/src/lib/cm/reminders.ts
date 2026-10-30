import { EditorView, ViewPlugin, type ViewUpdate } from '@codemirror/view';
import { convertLine } from '../reminder';

/** Replaces a «напомнить …» phrase in the line at `pos` with its ⏰ marker. */
function convertAt(view: EditorView, pos: number) {
  const doc = view.state.doc;
  if (pos > doc.length) return;
  const line = doc.lineAt(pos);
  const next = convertLine(line.text);
  if (next === null || next === line.text) return;
  view.dispatch({ changes: { from: line.from, to: line.to, insert: next }, userEvent: 'bedrock.reminder' });
}

/**
 * Turns «напомнить завтра в 9» into «⏰ 2026-10-31 09:00» once the line is
 * finished: when the cursor leaves it or the editor loses focus. Only lines
 * the user typed in are touched, so opening an old note never rewrites it.
 */
export const reminderPhrases = ViewPlugin.fromClass(
  class {
    /** A position inside the line last typed in. */
    edited: number | null = null;

    constructor(readonly view: EditorView) {}

    finish() {
      const pos = this.edited;
      this.edited = null;
      if (pos !== null) setTimeout(() => convertAt(this.view, pos));
    }

    update(u: ViewUpdate) {
      if (this.edited !== null && u.docChanged) this.edited = u.changes.mapPos(this.edited, -1);
      const doc = u.state.doc;
      const head = u.state.selection.main.head;
      if (this.edited !== null && doc.lineAt(Math.min(this.edited, doc.length)).number !== doc.lineAt(head).number) {
        this.finish();
      }
      if (u.transactions.some((tr) => tr.isUserEvent('input') || tr.isUserEvent('delete'))) this.edited = head;
      if (u.focusChanged && !u.view.hasFocus) this.finish();
    }
  },
);
