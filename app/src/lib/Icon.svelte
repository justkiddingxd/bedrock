<script lang="ts" module>
  // 16px line icons, 1.5px stroke.
  const ICONS: Record<string, string> = {
    sun: '<circle cx="8" cy="8" r="3"/><path d="M8 1v1.5M8 13.5V15M1 8h1.5M13.5 8H15M3 3l1 1M12 12l1 1M3 13l1-1M12 4l1-1"/>',
    inbox: '<path d="M2 9.5V13a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1V9.5l-2-6.5H4z"/><path d="M2 9.5h3.5l1 1.5h3l1-1.5H14"/>',
    calendar: '<rect x="2" y="3" width="12" height="11" rx="1.5"/><path d="M2 6.5h12M5.5 1.5v3M10.5 1.5v3"/>',
    notes: '<path d="M3.5 2.5h6.5l2.5 2.5v8.5h-9z"/><path d="M5.75 7h4.5M5.75 9.5h4.5M5.75 12h2.5"/>',
    doc: '<path d="M3.5 2.5h6.5l2.5 2.5v8.5h-9z"/>',
    folder: '<path d="M1.75 4.25v8a1 1 0 0 0 1 1h10.5a1 1 0 0 0 1-1v-6.5a1 1 0 0 0-1-1H7.5L6 3.25H2.75a1 1 0 0 0-1 1z"/>',
    search: '<circle cx="7" cy="7" r="4.75"/><path d="M10.5 10.5l3.25 3.25"/>',
    plus: '<path d="M8 3v10M3 8h10"/>',
    'chevron-down': '<path d="M4 6l4 4 4-4"/>',
    'chevron-up': '<path d="M4 10l4-4 4 4"/>',
    home: '<path d="M2.5 7.25L8 2.75l5.5 4.5v5.75a1 1 0 0 1-1 1H10V10H6v4H3.5a1 1 0 0 1-1-1z"/>',
    image: '<rect x="2" y="2.5" width="12" height="11" rx="1.5"/><circle cx="5.75" cy="6.25" r="1.25"/><path d="M14 10.5l-3.5-3.5L4 13.5"/>',
    repeat: '<path d="M2.5 7a4 4 0 0 1 4-4h6M10.5 1l2 2-2 2M13.5 9a4 4 0 0 1-4 4h-6M5.5 15l-2-2 2-2"/>',
    board: '<rect x="1.75" y="2.5" width="3.5" height="11" rx="1"/><rect x="6.25" y="2.5" width="3.5" height="7" rx="1"/><rect x="10.75" y="2.5" width="3.5" height="9" rx="1"/>',
    bold: '<path d="M4.5 2.75h4.25a2.6 2.6 0 0 1 0 5.2H4.5zM4.5 7.95h5a2.65 2.65 0 0 1 0 5.3h-5z" stroke-width="1.8"/>',
    italic: '<path d="M7 2.75h5M4 13.25h5M10 2.75L6 13.25"/>',
    strike: '<path d="M2.5 8h11M10.8 4.6C10.3 3.5 9.2 2.75 7.9 2.75c-1.8 0-3.1 1-3.1 2.4 0 .9.5 1.6 1.6 2.1M5 11.3c.5 1.2 1.7 1.95 3.1 1.95 1.8 0 3.1-1 3.1-2.45 0-.5-.15-.95-.45-1.3"/>',
    code: '<path d="M5.5 4.5L2 8l3.5 3.5M10.5 4.5L14 8l-3.5 3.5"/>',
    wikilink: '<path d="M5 2.75H3v10.5h2M11 2.75h2v10.5h-2M6.5 8h3"/>',
    list: '<path d="M6.5 4h7.5M6.5 8h7.5M6.5 12h7.5"/><circle cx="3" cy="4" r="1" fill="currentColor" stroke="none"/><circle cx="3" cy="8" r="1" fill="currentColor" stroke="none"/><circle cx="3" cy="12" r="1" fill="currentColor" stroke="none"/>',
    'list-ordered': '<path d="M7 4h7M7 8h7M7 12h7M2.5 3.2l1-.45V6M2.1 9.6c.2-.4.55-.6.95-.6.55 0 .95.35.95.85 0 .75-1.9 1.3-1.9 2.4h1.95"/>',
    quote: '<path d="M3 3.5v9M6 4.5h7.5M6 8h7.5M6 11.5h5"/>',
    'code-block': '<rect x="2" y="2.5" width="12" height="11" rx="1.5"/><path d="M6.5 6.5L5 8l1.5 1.5M9.5 6.5L11 8l-1.5 1.5"/>',
    table: '<rect x="2" y="2.5" width="12" height="11" rx="1.5"/><path d="M2 6.5h12M2 10h12M6.5 2.5v11"/>',
    minus: '<path d="M2.5 8h11"/>',
    cut: '<circle cx="4.5" cy="11.5" r="2"/><circle cx="11.5" cy="11.5" r="2"/><path d="M5.9 10L12 2.5M10.1 10L4 2.5"/>',
    paste: '<path d="M5.5 3H4a1 1 0 0 0-1 1v9a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V4a1 1 0 0 0-1-1h-1.5"/><rect x="5.5" y="1.75" width="5" height="2.5" rx="1"/>',
    'select-all': '<path d="M3 5.5V3h2.5M10.5 3H13v2.5M13 10.5V13h-2.5M5.5 13H3v-2.5"/><rect x="6" y="6" width="4" height="4" rx=".5"/>',
    external: '<path d="M9 2.75h4.25V7M13.25 2.75L7.5 8.5M12 9.5V12a1.25 1.25 0 0 1-1.25 1.25H4A1.25 1.25 0 0 1 2.75 12V5.25C2.75 4.56 3.31 4 4 4h2.5"/>',
    'chevron-left': '<path d="M10 3.5L5.5 8 10 12.5"/>',
    'chevron-right': '<path d="M6 3.5L10.5 8 6 12.5"/>',
    gear: '<circle cx="8" cy="8" r="2.25"/><path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4"/>',
    cloud: '<path d="M4.5 12.5h7a3 3 0 0 0 .4-6 4.25 4.25 0 0 0-8.1 1.1A2.5 2.5 0 0 0 4.5 12.5z"/>',
    check: '<path d="M3.5 8.5l3 3 6-7"/>',
    x: '<path d="M4 4l8 8M12 4l-8 8"/>',
    trash: '<path d="M2.5 4.5h11M6.5 4.5V3h3v1.5M4 4.5l.6 8.6a1 1 0 0 0 1 .9h4.8a1 1 0 0 0 1-.9l.6-8.6"/>',
    pin: '<path d="M6 2.5h4M7 2.5v3.5L4.5 9h7L9 6V2.5M8 9v4.5"/>',
    reveal: '<path d="M1.75 4.25v8a1 1 0 0 0 1 1h10.5a1 1 0 0 0 1-1v-6.5a1 1 0 0 0-1-1H7.5L6 3.25H2.75a1 1 0 0 0-1 1z"/><path d="M6.5 10.5l3-3M7 7.5h2.5V10"/>',
    pencil: '<path d="M10.5 2.5l3 3-8 8H2.5v-3z"/>',
    sidebar: '<rect x="2" y="2.5" width="12" height="11" rx="1.5"/><path d="M6 2.5v11"/>',
    'panel-right': '<rect x="2" y="2.5" width="12" height="11" rx="1.5"/><path d="M10 2.5v11"/>',
    enter: '<path d="M13 3v4.5a2 2 0 0 1-2 2H3.5M6 6.5l-3 3 3 3"/>',
    warn: '<path d="M8 2l6 11H2z"/><path d="M8 6.5v3M8 11.5v.01"/>',
    lock: '<rect x="3" y="7" width="10" height="7" rx="1.5"/><path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2"/>',
    link: '<path d="M6.5 9.5l3-3"/><path d="M7 4.5l1-1a2.5 2.5 0 0 1 3.5 3.5l-1 1M9 11.5l-1 1A2.5 2.5 0 0 1 4.5 9l1-1"/>',
    flag: '<path d="M3.5 14V2.5M3.5 2.5h8l-1.5 3 1.5 3h-8"/>',
    bell: '<path d="M4 11.5V7.5a4 4 0 0 1 8 0v4l1.2 1.5H2.8z"/><path d="M6.6 14.5h2.8"/>',
    people: '<circle cx="6" cy="5.5" r="2.3"/><path d="M1.8 13.5a4.2 4.2 0 0 1 8.4 0"/><path d="M10.5 3.4a2.3 2.3 0 0 1 0 4.3M12 9.6a4.2 4.2 0 0 1 2.2 3.9"/>',
    copy: '<rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M10.5 5.5V3.8A1.3 1.3 0 0 0 9.2 2.5H3.8a1.3 1.3 0 0 0-1.3 1.3v5.4a1.3 1.3 0 0 0 1.3 1.3h1.7"/>',
    send: '<path d="M14 2L7 9M14 2l-4.5 12L7 9 2 6.5z"/>',
    plug: '<path d="M6 2v3.5M10 2v3.5M4 5.5h8v2a4 4 0 0 1-8 0zM8 11.5V14"/>',
  };
  const FILLED: Record<string, string> = {
    more: '<circle cx="3.5" cy="8" r="1.2"/><circle cx="8" cy="8" r="1.2"/><circle cx="12.5" cy="8" r="1.2"/>',
  };
  /** The SVG markup of an icon, for code that builds its DOM itself (editor widgets, reading mode). */
  export function iconSvg(name: string, size = 16, weight = 1.5): string {
    return FILLED[name]
      ? `<svg width="${size}" height="${size}" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">${FILLED[name]}</svg>`
      : `<svg width="${size}" height="${size}" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="${weight}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name] ?? ''}</svg>`;
  }
</script>

<script lang="ts">
  let { name, size = 16, weight = 1.5 }: { name: string; size?: number; weight?: number } = $props();
  const svg = $derived(iconSvg(name, size, weight));
</script>

<span class="icon">{@html svg}</span>

<style>
  .icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
</style>
