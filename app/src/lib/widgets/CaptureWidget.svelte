<script lang="ts">
  let { onCapture }: { onCapture: (text: string) => Promise<void> } = $props();

  let text = $state('');
  let busy = $state(false);

  async function save() {
    const t = text.trim();
    if (!t || busy) return;
    busy = true;
    try {
      await onCapture(t);
      text = '';
    } finally {
      busy = false;
    }
  }
</script>

<textarea
  class="capture"
  bind:value={text}
  rows="3"
  placeholder="Мысль, ссылка, идея… Ctrl+Enter — сохранить"
  aria-label="Быстрая запись"
  onkeydown={(e) => {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      save();
    }
  }}
></textarea>
<div class="capture-foot">
  <span>Первая строка станет названием заметки</span>
  <button class="btn" disabled={!text.trim() || busy} onclick={save}>Сохранить</button>
</div>
