/**
 * The sound of a task getting done, synthesized (no sound files). Several
 * soft ones to choose from in the appearance settings, all quiet and
 * filtered of harsh highs.
 */

import { loadPrefs } from './prefs';

export type Sound = 'drop' | 'marimba' | 'chord' | 'tick' | 'off';

export const SOUNDS: { id: Sound; label: string }[] = [
  { id: 'drop', label: 'Капля' },
  { id: 'marimba', label: 'Маримба' },
  { id: 'chord', label: 'Аккорд' },
  { id: 'tick', label: 'Щелчок' },
  { id: 'off', label: 'Без звука' },
];

let ctx: AudioContext | null = null;

/** Everything goes through a quiet gain and a low-pass filter. */
function output(audio: AudioContext, level: number, cutoff = 3200): AudioNode {
  const gain = audio.createGain();
  gain.gain.value = level;
  const filter = audio.createBiquadFilter();
  filter.type = 'lowpass';
  filter.frequency.value = cutoff;
  gain.connect(filter).connect(audio.destination);
  return gain;
}

/** A tone with a fast attack and an exponential fade. */
function tone(audio: AudioContext, out: AudioNode, opts: { type?: OscillatorType; freq: number; at: number; length: number; level?: number; glideTo?: number }) {
  const { type = 'sine', freq, at, length, level = 1, glideTo } = opts;
  const osc = audio.createOscillator();
  const gain = audio.createGain();
  osc.type = type;
  osc.frequency.setValueAtTime(freq, at);
  if (glideTo) osc.frequency.exponentialRampToValueAtTime(glideTo, at + length * 0.45);
  gain.gain.setValueAtTime(0, at);
  gain.gain.linearRampToValueAtTime(level, at + 0.005);
  gain.gain.exponentialRampToValueAtTime(0.0001, at + length);
  osc.connect(gain).connect(out);
  osc.start(at);
  osc.stop(at + length + 0.02);
}

const PLAY: Record<Exclude<Sound, 'off'>, (audio: AudioContext, t: number) => void> = {
  // A small rising «bloop», like a bubble.
  drop: (audio, t) => {
    const out = output(audio, 0.22, 2400);
    tone(audio, out, { freq: 300, glideTo: 820, at: t, length: 0.16 });
  },
  // Two warm mallet notes; the high partial gives the wooden knock.
  marimba: (audio, t) => {
    const out = output(audio, 0.2);
    for (const [freq, at] of [
      [523.25, 0],
      [783.99, 0.085],
    ]) {
      tone(audio, out, { freq, at: t + at, length: 0.5 });
      tone(audio, out, { freq: freq * 4, at: t + at, length: 0.05, level: 0.25 });
    }
  },
  // A soft major arpeggio.
  chord: (audio, t) => {
    const out = output(audio, 0.12, 2600);
    [523.25, 659.25, 783.99].forEach((freq, i) => tone(audio, out, { type: 'triangle', freq, at: t + i * 0.035, length: 0.7, level: 0.8 }));
  },
  // A short, dry click.
  tick: (audio, t) => {
    const out = output(audio, 0.35, 5000);
    const noise = audio.createBuffer(1, Math.floor(audio.sampleRate * 0.03), audio.sampleRate);
    const data = noise.getChannelData(0);
    for (let i = 0; i < data.length; i++) data[i] = (Math.random() * 2 - 1) * (1 - i / data.length) ** 3;
    const src = audio.createBufferSource();
    src.buffer = noise;
    const band = audio.createBiquadFilter();
    band.type = 'bandpass';
    band.frequency.value = 1900;
    band.Q.value = 1.4;
    src.connect(band).connect(out);
    src.start(t);
    tone(audio, out, { freq: 1500, glideTo: 900, at: t, length: 0.03, level: 0.3 });
  },
};

export function playSound(sound: Sound) {
  if (sound === 'off') return;
  try {
    ctx ??= new AudioContext();
    PLAY[sound](ctx, ctx.currentTime + 0.01);
  } catch {
    // No audio device: stay silent.
  }
}

export function playDone() {
  playSound(loadPrefs().sound);
}
