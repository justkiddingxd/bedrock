import '@fontsource-variable/onest';
import './app.css';
import { mount } from 'svelte';
import { getCurrentWindow } from '@tauri-apps/api/window';
import App from './App.svelte';
import { applyPrefs, loadPrefs } from './lib/prefs';

applyPrefs(loadPrefs());

const app = mount(App, { target: document.getElementById('app')! });

// The window starts hidden so it never flashes an unstyled page.
requestAnimationFrame(() => getCurrentWindow().show());

export default app;
