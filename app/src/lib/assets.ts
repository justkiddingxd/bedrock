/** Files of the open vault as URLs the webview can show (the asset protocol may read inside the vault). */

import { convertFileSrc } from '@tauri-apps/api/core';
import { api } from './api';

let root = '';

export function setVaultRoot(path: string) {
  root = path;
  resolved.clear();
}

export function assetUrl(rel: string): string {
  const sep = root.includes('\\') ? '\\' : '/';
  return convertFileSrc(`${root}${sep}${rel.split('/').join(sep)}`);
}

const resolved = new Map<string, Promise<string | null>>();

/** The URL of a picture linked from note `from`: a web address as is, a vault file through the asset protocol. */
export function imageUrl(target: string, from: string): Promise<string | null> {
  if (/^(https?:|data:)/i.test(target)) return Promise.resolve(target);
  const key = `${from}|${target}`;
  let url = resolved.get(key);
  if (!url) {
    let name = target;
    try {
      name = decodeURIComponent(target);
    } catch {
      // Not percent-encoded.
    }
    url = api
      .followLink(name, from, false)
      .then((path) => (path ? assetUrl(path) : null))
      .catch(() => null);
    resolved.set(key, url);
    // A missing picture may appear later (synced, added): ask again next time.
    url.then((u) => u || resolved.delete(key));
  }
  return url;
}
