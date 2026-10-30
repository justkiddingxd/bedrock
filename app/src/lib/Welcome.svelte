<script lang="ts">
  import WindowControls from './WindowControls.svelte';
  import appIcon from '../assets/bedrock.svg';

  interface Props {
    recent: string[];
    onPick: () => void;
    onOpen: (path: string) => void;
  }
  let { recent, onPick, onOpen }: Props = $props();

  const name = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;
</script>

<div class="welcome">
  <div class="topbar" data-tauri-drag-region><WindowControls /></div>
  <div class="welcome-body" data-tauri-drag-region>
    <div class="welcome-box">
      <img src={appIcon} alt="" />
      <h1>Bedrock</h1>
      <p>Заметки, задачи и заметка дня в обычных markdown-файлах. Хранилище — это папка на диске, подойдёт и хранилище Obsidian.</p>
      <button class="btn primary wide" onclick={onPick}>Открыть папку как хранилище</button>
      {#if recent.length}
        <div class="recent-title">Недавние</div>
        {#each recent as path (path)}
          <button class="recent-row" onclick={() => onOpen(path)}>
            <span class="rn">{name(path)}</span>
            <span class="rp">{path}</span>
          </button>
        {/each}
      {/if}
    </div>
  </div>
</div>
