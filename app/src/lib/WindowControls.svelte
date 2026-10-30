<script lang="ts" module>
  /** macOS draws its own traffic lights over the content (titleBarStyle: Overlay). */
  export const IS_MAC = navigator.userAgent.includes('Mac');
</script>

<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';

  const win = getCurrentWindow();
</script>

{#if !IS_MAC}
  <div class="caption">
    <button aria-label="Свернуть" onclick={() => win.minimize()}>
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true"><path d="M0 5.5h10" /></svg>
    </button>
    <button aria-label="Развернуть" onclick={() => win.toggleMaximize()}>
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true"><rect x="0.5" y="0.5" width="9" height="9" /></svg>
    </button>
    <button class="close" aria-label="Закрыть" onclick={() => win.close()}>
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true"><path d="M0 0l10 10M10 0L0 10" /></svg>
    </button>
  </div>
{/if}
