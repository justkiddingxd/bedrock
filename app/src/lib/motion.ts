/** Animation timing shared by Svelte transitions; CSS animations follow the same setting in app.css. */

import { cubicOut } from 'svelte/easing';

const reduce = window.matchMedia('(prefers-reduced-motion: reduce)');

/** `n` milliseconds, or none when the system asks for less motion. */
export const ms = (n: number) => (reduce.matches ? 0 : n);

export const ease = cubicOut;
