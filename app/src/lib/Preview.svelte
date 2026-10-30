<script lang="ts">
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { plural } from './api';
  import { FOLD_KEEP, foldedBlocks } from './codefold';
  import { iconSvg } from './Icon.svelte';

  interface Props {
    html: string;
    /** The note, to open code blocks folded in the editor folded here too. */
    path: string;
    root: string;
    onOpenLink: (target: string, heading: string | null) => void;
    onTag: (tag: string) => void;
    onToggleTask: (line: number) => void;
  }
  let { html, path, root, onOpenLink, onTag, onToggleTask }: Props = $props();
  let el: HTMLDivElement;

  // Local images are rendered as <img data-src="vault/path">; map them to the
  // asset protocol, which may only read inside the vault.
  $effect(() => {
    void html;
    const sep = root.includes('\\') ? '\\' : '/';
    el?.querySelectorAll<HTMLImageElement>('img[data-src]').forEach((img) => {
      const rel = (img.dataset.src ?? '').split('/').join(sep);
      img.src = convertFileSrc(`${root}${sep}${rel}`);
    });
  });

  // Code blocks get «Копировать» and, when long, «Свернуть» / «Развернуть».
  $effect(() => {
    void html;
    const folded = foldedBlocks(path);
    el?.querySelectorAll('pre').forEach((pre, i) => {
      if (pre.parentElement?.classList.contains('code-wrap')) return;
      const lines = (pre.textContent ?? '').replace(/\n$/, '').split('\n').length;
      const wrap = document.createElement('div');
      wrap.className = 'code-wrap';
      wrap.dataset.hidden = String(lines - FOLD_KEEP);
      pre.replaceWith(wrap);
      const tools = document.createElement('div');
      tools.className = 'code-tools';
      if (lines > FOLD_KEEP) tools.append(codeButton('fold', ''));
      const copy = codeButton('copy', '');
      copy.classList.add('icon');
      copy.innerHTML = iconSvg('copy', 14);
      copy.title = 'Скопировать весь код';
      copy.setAttribute('aria-label', 'Скопировать код');
      tools.append(copy);
      wrap.append(tools, pre);
      if (lines > FOLD_KEEP) {
        const more = codeButton('fold', `··· ещё ${lines - FOLD_KEEP} ${plural(lines - FOLD_KEEP, ['строка', 'строки', 'строк'])}`);
        more.classList.add('code-more');
        wrap.append(more);
        setFolded(wrap, folded.has(i));
      }
    });
  });

  function codeButton(action: string, label: string) {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'code-btn';
    b.dataset.action = action;
    b.textContent = label;
    return b;
  }

  function setFolded(wrap: Element, on: boolean) {
    wrap.classList.toggle('folded', on);
    const fold = wrap.querySelector<HTMLElement>('.code-tools [data-action="fold"]');
    if (fold) fold.textContent = on ? 'Развернуть' : 'Свернуть';
  }

  function codeAction(btn: HTMLElement) {
    const wrap = btn.closest('.code-wrap');
    if (!wrap) return;
    if (btn.dataset.action === 'fold') {
      setFolded(wrap, !wrap.classList.contains('folded'));
      return;
    }
    // The whole code, also when only the first lines are shown.
    navigator.clipboard.writeText(wrap.querySelector('pre')?.textContent?.replace(/\n$/, '') ?? '').then(
      () => {
        btn.innerHTML = iconSvg('check', 14);
        btn.classList.add('done');
        btn.title = 'Скопировано';
        setTimeout(() => {
          btn.innerHTML = iconSvg('copy', 14);
          btn.classList.remove('done');
          btn.title = 'Скопировать весь код';
        }, 1500);
      },
      () => (btn.title = 'Не удалось скопировать'),
    );
  }

  function onclick(e: MouseEvent) {
    const t = e.target as HTMLElement;
    const codeBtn = t.closest<HTMLElement>('.code-btn');
    if (codeBtn) {
      codeAction(codeBtn);
      return;
    }
    if (t instanceof HTMLInputElement && t.classList.contains('task-checkbox')) {
      e.preventDefault();
      onToggleTask(Number(t.dataset.line));
      return;
    }
    const a = t.closest('a');
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute('href') ?? '';
    if (a.classList.contains('wikilink')) onOpenLink(a.dataset.target ?? '', a.dataset.heading || null);
    else if (a.classList.contains('tag')) onTag(`#${a.dataset.tag ?? ''}`);
    else if (/^(https?:|mailto:)/i.test(href)) openUrl(href);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="preview markdown" bind:this={el} {onclick}>{@html html}</div>
