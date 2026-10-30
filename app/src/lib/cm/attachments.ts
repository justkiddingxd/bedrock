/**
 * Pictures and files in notes: pasted (Ctrl+V a screenshot), dropped from the
 * file manager or picked. They are saved to the vault's «Вложения» folder and
 * the note gets `![[file.png]]` (a picture) or `[[file.pdf]]` (other files).
 */

import { EditorView } from '@codemirror/view';
import { api, errText, type Attached } from '../api';

const pad = (n: number) => String(n).padStart(2, '0');

/** Clipboard pictures come as «image.png»: give them a dated name. */
function pastedName(file: File): string {
  if (file.name && !/^image\.\w+$/i.test(file.name)) return file.name;
  const d = new Date();
  const ext = file.type.split('/')[1]?.replace('jpeg', 'jpg').replace('svg+xml', 'svg') || 'png';
  return `Вставка ${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}-${pad(d.getMinutes())}-${pad(d.getSeconds())}.${ext}`;
}

function base64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(',')[1] ?? '');
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

/** Saves the files one by one and puts their links at `at`, each on its own line. */
export async function insertAttachments(
  view: EditorView,
  files: (() => Promise<Attached>)[],
  at: number,
  onError: (message: string) => void,
): Promise<void> {
  const links: string[] = [];
  for (const save of files) {
    try {
      const a = await save();
      links.push(a.image ? `![[${a.link}]]` : `[[${a.link}]]`);
    } catch (e) {
      onError(`Не удалось добавить файл: ${errText(e)}`);
    }
  }
  if (!links.length) return;
  const pos = Math.min(at, view.state.doc.length);
  const line = view.state.doc.lineAt(pos);
  const lead = pos > line.from ? '\n' : '';
  const trail = pos < line.to ? '\n' : '';
  const text = `${lead}${links.join('\n')}${trail}`;
  view.dispatch({
    changes: { from: pos, insert: text },
    selection: { anchor: pos + text.length },
    scrollIntoView: true,
    userEvent: 'input.paste',
  });
  view.focus();
}

export function pastedFiles(onError: (message: string) => void) {
  return EditorView.domEventHandlers({
    paste(e, view) {
      const files = [...(e.clipboardData?.files ?? [])];
      if (!files.length) return false;
      e.preventDefault();
      void insertAttachments(
        view,
        files.map((f) => async () => api.saveAttachment(pastedName(f), await base64(f))),
        view.state.selection.main.head,
        onError,
      );
      return true;
    },
  });
}
