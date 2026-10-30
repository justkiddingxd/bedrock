<script lang="ts">
  import { ACCENTS, loadPrefs, savePrefs, type Prefs } from './prefs';

  let { x, y, onClose }: { x: number; y: number; onClose: () => void } = $props();
  let prefs = $state<Prefs>(loadPrefs());
  const dark = $derived(
    prefs.theme === 'dark' || (prefs.theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches),
  );

  function set<K extends keyof Prefs>(key: K, value: Prefs[K]) {
    prefs[key] = value;
    savePrefs({ ...prefs });
  }

  const THEMES = [
    { id: 'dark', label: 'Тёмная' },
    { id: 'light', label: 'Светлая' },
    { id: 'system', label: 'Авто' },
  ] as const;
  const SIZES = [
    { id: 's', label: 'Мельче' },
    { id: 'm', label: 'Обычный' },
    { id: 'l', label: 'Крупнее' },
  ] as const;
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onClose()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={onClose}></div>
<div class="popover" style:left="{x}px" style:bottom="{y}px" role="dialog" tabindex="-1" aria-label="Оформление">
  <section>
    <h4>Тема</h4>
    <div class="segmented">
      {#each THEMES as t (t.id)}
        <button class:active={prefs.theme === t.id} onclick={() => set('theme', t.id)}>{t.label}</button>
      {/each}
    </div>
  </section>
  <section>
    <h4>Акцент</h4>
    <div class="accents">
      {#each ACCENTS as a (a.id)}
        <button
          class="accent-dot"
          class:active={prefs.accent === a.id}
          style:background={dark ? a.dark : a.light}
          title={a.name}
          aria-label={a.name}
          onclick={() => set('accent', a.id)}
        ></button>
      {/each}
    </div>
  </section>
  <section>
    <h4>Размер текста</h4>
    <div class="segmented">
      {#each SIZES as s (s.id)}
        <button class:active={prefs.textSize === s.id} onclick={() => set('textSize', s.id)}>{s.label}</button>
      {/each}
    </div>
  </section>
  <section>
    <h4>Редактор</h4>
    <label class="check-row">
      <input type="checkbox" checked={prefs.livePreview} onchange={(e) => set('livePreview', e.currentTarget.checked)} />
      <span>Скрывать разметку<small>Символы # ** [[ ]] видны только в строке, которую правите</small></span>
    </label>
  </section>
  <section>
    <h4>Звуки</h4>
    <label class="check-row">
      <input type="checkbox" checked={prefs.sounds} onchange={(e) => set('sounds', e.currentTarget.checked)} />
      <span>Звук при выполнении задачи</span>
    </label>
  </section>
  <section>
    <h4>Боковая панель</h4>
    <label class="check-row">
      <input type="checkbox" checked={prefs.showTags} onchange={(e) => set('showTags', e.currentTarget.checked)} />
      <span>Метки<small>Слова с # из заметок, например #идея</small></span>
    </label>
  </section>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 54;
  }
</style>
