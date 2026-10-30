/// <reference types="svelte" />
/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** URL of the shared sync server offered as a preset in the sync dialog. */
  readonly VITE_SHARED_SERVER?: string;
}
