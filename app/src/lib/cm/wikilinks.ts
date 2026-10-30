import {
  Decoration,
  EditorView,
  MatchDecorator,
  ViewPlugin,
  WidgetType,
  type DecorationSet,
  type ViewUpdate,
} from '@codemirror/view';
import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete';
import type { NoteRef } from '../api';
import { setLinesDone } from './format';
import { addDays, dayLabel, localDate, weekdayName } from '../dates';
import { nextWeekday } from '../quick';

class CheckboxWidget extends WidgetType {
  constructor(readonly checked: boolean) {
    super();
  }
  eq(other: CheckboxWidget) {
    return other.checked === this.checked;
  }
  toDOM() {
    const wrap = document.createElement('span');
    wrap.className = 'cm-task-checkbox';
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.checked = this.checked;
    box.tabIndex = -1;
    wrap.appendChild(box);
    return wrap;
  }
  ignoreEvent() {
    return false;
  }
}

const bracket = Decoration.mark({ class: 'cm-wikilink-bracket' });

/** "[[Target]]": brackets dimmed, target styled as a link. */
const linkMatcher = new MatchDecorator({
  regexp: /(!?\[\[)([^\[\]\n]+?)\]\]/g,
  decorate: (add, from, to, m) => {
    const inner = from + m[1].length;
    add(from, inner, bracket);
    add(inner, to - 2, Decoration.mark({ class: 'cm-wikilink', attributes: { 'data-target': m[2] } }));
    add(to - 2, to, bracket);
  },
});

const tagMatcher = new MatchDecorator({
  regexp: /(^|[\s(,;])(#[\p{L}\p{N}_\/-]*[\p{L}_\/-][\p{L}\p{N}_\/-]*)/gu,
  decorate: (add, from, to, m) => add(from + m[1].length, to, Decoration.mark({ class: 'cm-tag' })),
});

/** "- [ ]" becomes a checkbox (the bullet hides with it); "1. [ ]" keeps its number. */
const taskMatcher = new MatchDecorator({
  regexp: /^(\s*)(?:([-*+])|(\d+[.)]))(\s+)\[([ xX])\]/g,
  decorate: (add, from, to, m) => {
    const start = m[2] ? from + m[1].length : from + m[1].length + m[3].length + m[4].length;
    add(start, to, Decoration.replace({ widget: new CheckboxWidget(m[5] !== ' ') }));
  },
});

function matcherPlugin(matcher: MatchDecorator, atomic = false) {
  return ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = matcher.createDeco(view);
      }
      update(u: ViewUpdate) {
        this.decorations = matcher.updateDeco(u, this.decorations);
      }
    },
    {
      decorations: (v) => v.decorations,
      provide: atomic
        ? (plugin) => EditorView.atomicRanges.of((view) => view.plugin(plugin)?.decorations ?? Decoration.none)
        : undefined,
    },
  );
}

/** Wikilink and tag highlighting, clickable task checkboxes, Ctrl+click navigation. */
export function wikilinks(opts: { onOpenLink: (raw: string) => void; onTag: (tag: string) => void }) {
  return [
    matcherPlugin(linkMatcher),
    matcherPlugin(tagMatcher),
    matcherPlugin(taskMatcher, true),
    EditorView.domEventHandlers({
      mousedown(e, view) {
        const target = e.target as HTMLElement;
        if (target instanceof HTMLInputElement && target.closest('.cm-task-checkbox')) {
          const start = view.posAtDOM(target);
          const pos = start + Math.max(0, view.state.sliceDoc(start, start + 8).indexOf('['));
          const box = view.state.sliceDoc(pos, pos + 3);
          if (/^\[.\]$/.test(box)) void setLinesDone(view, [view.state.doc.lineAt(pos).number], !/^\[[xX-]\]$/.test(box));
          e.preventDefault();
          return true;
        }
        if (e.ctrlKey || e.metaKey) {
          const link = target.closest<HTMLElement>('.cm-wikilink');
          if (link?.dataset.target) {
            opts.onOpenLink(link.dataset.target);
            e.preventDefault();
            return true;
          }
          const tag = target.closest('.cm-tag');
          if (tag?.textContent) {
            opts.onTag(tag.textContent.trim());
            e.preventDefault();
            return true;
          }
        }
        return false;
      },
    }),
  ];
}

/** "@завтра", "@пт"... in the text become a task due date: "📅 2026-10-30". */
export function dateCompletion() {
  return (ctx: CompletionContext): CompletionResult | null => {
    const m = ctx.matchBefore(/(?:^|\s)@[\p{L}\d.]*$/u);
    if (!m) return null;
    const today = localDate();
    const days: [string, string][] = [
      ['сегодня', today],
      ['завтра', addDays(today, 1)],
      ['послезавтра', addDays(today, 2)],
      ...([1, 2, 3, 4, 5, 6, 0] as const).map((dow): [string, string] => {
        const day = nextWeekday(today, dow);
        return [weekdayName(day).toLowerCase(), day];
      }),
      ['через неделю', addDays(today, 7)],
    ];
    return {
      from: m.from + m.text.indexOf('@'),
      options: days.map(([label, day]) => ({ label: `@${label}`, detail: dayLabel(day), apply: `📅 ${day}` })),
      validFor: /^@[\p{L}\d.]*$/u,
    };
  };
}

/** Ctrl+L: plain line -> "- [ ] task", list item -> task, task -> toggles done. */
export function toggleTask(view: EditorView): boolean {
  const seen = new Set<number>();
  const changes: { from: number; to?: number; insert: string }[] = [];
  const toggles: { open: number[]; closed: number[] } = { open: [], closed: [] };
  for (const range of view.state.selection.ranges) {
    const line = view.state.doc.lineAt(range.head);
    if (seen.has(line.number)) continue;
    seen.add(line.number);
    const task = /^(\s*(?:[-*+]|\d+[.)])\s+\[)(.)\]/.exec(line.text);
    const item = /^(\s*(?:[-*+]|\d+[.)])\s+)/.exec(line.text);
    if (task) {
      // Through the core, like a click on the checkbox: ✅ date and the next occurrence.
      (/[xX-]/.test(task[2]) ? toggles.closed : toggles.open).push(line.number);
    } else if (item) {
      changes.push({ from: line.from + item[1].length, insert: '[ ] ' });
    } else {
      changes.push({ from: line.from + (/^\s*/.exec(line.text)?.[0].length ?? 0), insert: '- [ ] ' });
    }
  }
  if (changes.length) view.dispatch({ changes });
  void setLinesDone(view, toggles.open, true).then(() => setLinesDone(view, toggles.closed, false));
  return true;
}

/** Autocompletes note names after "[[". */
export function wikilinkCompletion(getNotes: () => NoteRef[]) {
  return (ctx: CompletionContext): CompletionResult | null => {
    const m = ctx.matchBefore(/\[\[[^\[\]\n|#]*$/);
    if (!m) return null;
    const options: Completion[] = getNotes().map((n) => ({
      label: n.linkText,
      detail: n.title !== n.linkText.split('/').pop() ? n.title : undefined,
      apply: (view, _completion, from, to) => {
        const close = view.state.sliceDoc(to, to + 2) === ']]' ? '' : ']]';
        view.dispatch({
          changes: { from, to, insert: n.linkText + close },
          selection: { anchor: from + n.linkText.length + 2 },
        });
      },
    }));
    return { from: m.from + 2, options, validFor: /^[^\[\]\n|#]*$/ };
  };
}
