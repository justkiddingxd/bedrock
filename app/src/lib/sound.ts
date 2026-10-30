/** A soft two-note chime when a task gets done, synthesized (no sound files). Off in the appearance settings. */

import { loadPrefs } from './prefs';

let ctx: AudioContext | null = null;

export function playDone() {
  if (!loadPrefs().sounds) return;
  try {
    ctx ??= new AudioContext();
    const audio = ctx;
    const t0 = audio.currentTime;
    const out = audio.createGain();
    out.gain.value = 0.16;
    out.connect(audio.destination);
    // A5 then E6, a fifth apart; a quiet octave above gives a bell-like tone.
    for (const [freq, at] of [
      [880, 0],
      [1318.5, 0.08],
    ]) {
      for (const [mult, level] of [
        [1, 1],
        [2, 0.18],
      ]) {
        const osc = audio.createOscillator();
        const gain = audio.createGain();
        osc.type = 'sine';
        osc.frequency.value = freq * mult;
        gain.gain.setValueAtTime(0, t0 + at);
        gain.gain.linearRampToValueAtTime(level, t0 + at + 0.006);
        gain.gain.exponentialRampToValueAtTime(0.0001, t0 + at + 0.42);
        osc.connect(gain).connect(out);
        osc.start(t0 + at);
        osc.stop(t0 + at + 0.45);
      }
    }
  } catch {
    // No audio device: stay silent.
  }
}
