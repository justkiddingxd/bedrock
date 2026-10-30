/**
 * Drag and drop with the mouse. HTML5 drag events are not used: the desktop
 * webview hands them to the window (for dropping files from the system).
 * Items are dragged with `use:draggable={payload}`; places that take them use
 * `use:droppable={spec}`. A drag starts after the mouse moves a few pixels,
 * so a plain click still clicks; Escape cancels.
 */

import { displayName, type TaskItem } from './api';

export type Payload =
  | { kind: 'note'; path: string }
  | { kind: 'folder'; path: string }
  | { kind: 'task'; task: TaskItem }
  | { kind: 'widget'; id: string; label: string };

export interface DropSpec {
  accepts: (p: Payload) => boolean;
  onDrop: (p: Payload) => void;
  /** Called once the pointer has stayed over the target for a while (opens a folder). */
  onHold?: () => void;
}

/** Drop targets the app hands to its screens. */
export interface Dnd {
  /** Notes and folders dropped here move into `folder` ('' = no folder, the vault root). */
  intoFolder: (folder: string, onHold?: () => void) => DropSpec;
  /** Tasks dropped here get this due date; `null` takes it away. */
  onDay: (day: string | null) => DropSpec;
  /** Notes dropped here are pinned, before `before` or at the end. */
  pin: (before: string | null) => DropSpec;
}

const THRESHOLD = 5;
const HOLD_MS = 700;
const targets = new WeakMap<HTMLElement, DropSpec>();

let drag: { payload: Payload; ghost: HTMLElement; over: HTMLElement | null; holdTimer?: ReturnType<typeof setTimeout> } | null = null;

function label(p: Payload): string {
  if (p.kind === 'task') return p.task.text;
  if (p.kind === 'widget') return p.label;
  return p.kind === 'note' ? displayName(p.path) : (p.path.split('/').pop() ?? p.path);
}

function targetAt(x: number, y: number, payload: Payload): HTMLElement | null {
  for (let el = document.elementFromPoint(x, y) as HTMLElement | null; el; el = el.parentElement) {
    const spec = targets.get(el);
    if (spec?.accepts(payload)) return el;
  }
  return null;
}

function start(payload: Payload) {
  const ghost = document.createElement('div');
  ghost.className = 'drag-ghost';
  ghost.textContent = label(payload);
  document.body.append(ghost);
  document.documentElement.dataset.drag = payload.kind;
  window.getSelection()?.removeAllRanges();
  drag = { payload, ghost, over: null };
}

function track(x: number, y: number) {
  if (!drag) return;
  drag.ghost.style.transform = `translate(${x + 14}px, ${y + 12}px)`;
  const over = targetAt(x, y, drag.payload);
  if (over === drag.over) return;
  drag.over?.classList.remove('drop-over');
  clearTimeout(drag.holdTimer);
  drag.over = over;
  if (!over) return;
  over.classList.add('drop-over');
  const hold = targets.get(over)?.onHold;
  if (hold) drag.holdTimer = setTimeout(hold, HOLD_MS);
}

function finish(drop: boolean) {
  if (!drag) return;
  const { payload, ghost, over, holdTimer } = drag;
  drag = null;
  clearTimeout(holdTimer);
  ghost.remove();
  over?.classList.remove('drop-over');
  delete document.documentElement.dataset.drag;
  if (drop && over) targets.get(over)?.onDrop(payload);
}

/** Swallows the click that follows the mouse button release after a drag. */
function swallowClick() {
  const stop = (e: Event) => {
    e.preventDefault();
    e.stopPropagation();
  };
  window.addEventListener('click', stop, true);
  setTimeout(() => window.removeEventListener('click', stop, true), 0);
}

export function draggable(node: HTMLElement, payload: Payload) {
  let current = payload;
  function down(e: PointerEvent) {
    if (e.button !== 0 || (e.target as HTMLElement).closest('input, textarea, [contenteditable="true"]')) return;
    const x0 = e.clientX;
    const y0 = e.clientY;
    let moved = false;
    const move = (ev: PointerEvent) => {
      if (!moved) {
        if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < THRESHOLD) return;
        moved = true;
        start(current);
      }
      track(ev.clientX, ev.clientY);
    };
    const key = (ev: KeyboardEvent) => {
      if (ev.key !== 'Escape' || !drag) return;
      ev.preventDefault();
      ev.stopPropagation();
      finish(false);
    };
    const up = () => {
      window.removeEventListener('pointermove', move, true);
      window.removeEventListener('pointerup', up, true);
      window.removeEventListener('keydown', key, true);
      if (!moved) return;
      swallowClick();
      finish(true);
    };
    window.addEventListener('pointermove', move, true);
    window.addEventListener('pointerup', up, true);
    window.addEventListener('keydown', key, true);
  }
  node.addEventListener('pointerdown', down);
  return {
    update(p: Payload) {
      current = p;
    },
    destroy() {
      node.removeEventListener('pointerdown', down);
    },
  };
}

export function droppable(node: HTMLElement, spec: DropSpec | null) {
  const set = (s: DropSpec | null) => (s ? targets.set(node, s) : targets.delete(node));
  set(spec);
  return {
    update: set,
    destroy() {
      targets.delete(node);
    },
  };
}
