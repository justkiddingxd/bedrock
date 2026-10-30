/** What the main area shows. */
export type View =
  | { kind: 'today'; date: string }
  | { kind: 'note'; path: string; line?: number }
  | { kind: 'all' }
  | { kind: 'inbox' }
  | { kind: 'upcoming' }
  | { kind: 'folder'; path: string }
  | { kind: 'tag'; tag: string };

export interface Folder {
  path: string;
  name: string;
  count: number;
}

/** Same screen, ignoring the line a note was opened at. */
export function sameView(a: View, b: View): boolean {
  const strip = (v: View) => (v.kind === 'note' ? { kind: v.kind, path: v.path } : v);
  return JSON.stringify(strip(a)) === JSON.stringify(strip(b));
}
