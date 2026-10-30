/** Shared (public) sync server, set at build time: VITE_SHARED_SERVER=https://... */
export const SHARED_SERVER: string = import.meta.env.VITE_SHARED_SERVER ?? '';

/** Default address suggested for a self-hosted server. */
export const SELF_HOSTED_DEFAULT = 'http://localhost:8787';
