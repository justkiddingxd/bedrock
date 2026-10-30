import { StateEffect, StateField, type EditorState } from '@codemirror/state';
import {
  Decoration,
  EditorView,
  MatchDecorator,
  ViewPlugin,
  WidgetType,
  type DecorationSet,
  type ViewUpdate,
} from '@codemirror/view';

// ------------------------------------------------------------ frontmatter

/**
 * The YAML frontmatter is shown as properties above the editor, so in the
 * text it collapses into one line; clicking it reveals the source.
 */
const reveal = StateEffect.define<boolean>();

function frontmatterEnd(state: EditorState): { to: number; fields: number } | null {
  const doc = state.doc;
  if (doc.lines < 2 || doc.line(1).text.trimEnd() !== '---') return null;
  for (let i = 2; i <= Math.min(doc.lines, 100); i++) {
    const t = doc.line(i).text.trimEnd();
    if (t === '---' || t === '...') {
      let fields = 0;
      for (let j = 2; j < i; j++) if (/^[^\s#-][^:]*:/.test(doc.line(j).text)) fields++;
      return { to: doc.line(i).to, fields };
    }
  }
  return null;
}

class FrontmatterWidget extends WidgetType {
  constructor(readonly fields: number) {
    super();
  }
  eq(other: FrontmatterWidget) {
    return other.fields === this.fields;
  }
  toDOM(view: EditorView) {
    const el = document.createElement('button');
    el.className = 'cm-frontmatter';
    el.type = 'button';
    el.textContent = `Свойства (${this.fields}) — показать исходник`;
    el.onmousedown = (e) => {
      e.preventDefault();
      view.dispatch({ effects: reveal.of(true) });
    };
    return el;
  }
  ignoreEvent() {
    return true;
  }
}

function buildFrontmatter(state: EditorState, revealed: boolean): DecorationSet {
  const end = revealed ? null : frontmatterEnd(state);
  if (!end) return Decoration.none;
  return Decoration.set([Decoration.replace({ widget: new FrontmatterWidget(end.fields), block: true }).range(0, end.to)]);
}

const frontmatterField = StateField.define<{ revealed: boolean; deco: DecorationSet }>({
  create: (state) => ({ revealed: false, deco: buildFrontmatter(state, false) }),
  update(value, tr) {
    let revealed = value.revealed;
    for (const e of tr.effects) if (e.is(reveal)) revealed = e.value;
    if (!tr.docChanged && revealed === value.revealed) return value;
    return { revealed, deco: buildFrontmatter(tr.state, revealed) };
  },
  provide: (f) => [
    EditorView.decorations.from(f, (v) => v.deco),
    EditorView.atomicRanges.of((view) => view.state.field(f).deco),
  ],
});

// ------------------------------------------------------------ task markers

const ICONS: Record<string, string> = {
  due: '<rect x="2" y="3" width="12" height="11" rx="1.5"/><path d="M2 6.5h12M5.5 1.5v3M10.5 1.5v3"/>',
  scheduled: '<circle cx="8" cy="8" r="6"/><path d="M8 4.5V8l2.5 1.5"/>',
  done: '<path d="M3.5 8.5l3 3 6-7"/>',
  flag: '<path d="M3.5 14V2.5M3.5 2.5h8l-1.5 3 1.5 3h-8"/>',
  alarm: '<path d="M4 11.5V7.5a4 4 0 0 1 8 0v4l1.2 1.5H2.8z"/><path d="M6.6 14.5h2.8"/>',
  repeat: '<path d="M2.5 7a4 4 0 0 1 4-4h6M10.5 1l2 2-2 2M13.5 9a4 4 0 0 1-4 4h-6M5.5 15l-2-2 2-2"/>',
};

class MarkerWidget extends WidgetType {
  constructor(
    readonly kind: string,
    readonly cls: string,
    readonly label: string,
  ) {
    super();
  }
  eq(other: MarkerWidget) {
    return other.kind === this.kind && other.cls === this.cls;
  }
  toDOM() {
    const el = document.createElement('span');
    el.className = `cm-marker ${this.cls}`;
    el.title = this.label;
    el.innerHTML = `<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[this.kind]}</svg>`;
    return el;
  }
}

const DATE_KIND: Record<string, [string, string]> = {
  '📅': ['due', 'срок'],
  '⏳': ['scheduled', 'запланировано'],
  '✅': ['done', 'выполнено'],
};
const PRIORITY: Record<string, [string, string]> = {
  '⏫': ['cm-prio-high', 'высокий приоритет'],
  '🔺': ['cm-prio-high', 'высокий приоритет'],
  '🔼': ['cm-prio-medium', 'средний приоритет'],
  '🔽': ['cm-prio-low', 'низкий приоритет'],
  '⏬': ['cm-prio-low', 'низкий приоритет'],
};

const markerMatcher = new MatchDecorator({
  regexp: /(?:(📅|⏳|✅)\u{FE0F}?(\s*)(\d{4}-\d{2}-\d{2}))|(?:⏰\u{FE0F}?\s*(\d{4}-\d{2}-\d{2}[ T]\d{1,2}:\d{2}))|([⏫🔺🔼🔽⏬])\u{FE0F}?|(?:🔁\u{FE0F}?\s*([^📅⏳✅🛫➕⏰⏫🔼🔽🔺⏬#\n]*[^📅⏳✅🛫➕⏰⏫🔼🔽🔺⏬#\n\s]))/gu,
  decorate: (add, from, to, m) => {
    if (m[1]) {
      const [kind, label] = DATE_KIND[m[1]];
      const dateFrom = to - m[3].length;
      add(from, dateFrom, Decoration.replace({ widget: new MarkerWidget(kind, `cm-marker-${kind}`, label) }));
      add(dateFrom, to, Decoration.mark({ class: kind === 'done' ? 'cm-date cm-date-done' : 'cm-date' }));
    } else if (m[4]) {
      const stampFrom = to - m[4].length;
      add(from, stampFrom, Decoration.replace({ widget: new MarkerWidget('alarm', 'cm-marker-alarm', 'напоминание') }));
      add(stampFrom, to, Decoration.mark({ class: 'cm-date cm-date-alarm' }));
    } else if (m[5]) {
      const [cls, label] = PRIORITY[m[5]];
      add(from, to, Decoration.replace({ widget: new MarkerWidget('flag', cls, label) }));
    } else if (m[6]) {
      const ruleFrom = to - m[6].length;
      add(from, ruleFrom, Decoration.replace({ widget: new MarkerWidget('repeat', 'cm-marker-repeat', 'повторяется') }));
      add(ruleFrom, to, Decoration.mark({ class: 'cm-date cm-date-repeat' }));
    }
  },
});

const markerPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = markerMatcher.createDeco(view);
    }
    update(u: ViewUpdate) {
      this.decorations = markerMatcher.updateDeco(u, this.decorations);
    }
  },
  { decorations: (v) => v.decorations },
);

export const decorations = [frontmatterField, markerPlugin];
