/**
 * Code blocks folded in the editor, per note (block numbers in order of the
 * note), so reading mode opens them folded too. Kept for the session only.
 */

const folded = new Map<string, Set<number>>();

export const foldedBlocks = (path: string): Set<number> => folded.get(path) ?? new Set();

export function setFoldedBlocks(path: string, blocks: Set<number>) {
  if (blocks.size) folded.set(path, blocks);
  else folded.delete(path);
}

/** Lines of a code block to keep in view when it is folded. */
export const FOLD_KEEP = 3;
