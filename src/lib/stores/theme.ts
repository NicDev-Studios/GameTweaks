import { writable } from 'svelte/store';
import { getThemePreference, setThemePreference } from '$lib/api/app';

export type ThemeMode = 'system' | 'dark' | 'light';

function applyTheme(mode: ThemeMode) {
  const root = document.documentElement;
  root.dataset.theme = mode;
}

function createThemeStore() {
  const store = writable<ThemeMode>('system');
  const { subscribe, set } = store;
  let persistedMode: ThemeMode = 'system';
  let revision = 0;
  let writeQueue = Promise.resolve();

  return {
    subscribe,
    async init() {
      const initRevision = revision;
      const mode = await getThemePreference().catch(() => 'system' as ThemeMode);
      if (initRevision !== revision) return;

      persistedMode = mode;
      set(mode);
      applyTheme(mode);
    },
    async set(mode: ThemeMode) {
      const requestRevision = ++revision;
      set(mode);
      applyTheme(mode);

      const write = writeQueue.then(() => setThemePreference(mode));
      writeQueue = write.then(
        () => undefined,
        () => undefined
      );

      try {
        const savedMode = await write;
        persistedMode = savedMode;
        if (requestRevision !== revision) return;

        set(savedMode);
        applyTheme(savedMode);
      } catch {
        if (requestRevision !== revision) return;

        set(persistedMode);
        applyTheme(persistedMode);
      }
    }
  };
}

export const themeStore = createThemeStore();
