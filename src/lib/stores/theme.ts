import { writable } from 'svelte/store';
import { getThemePreference, setThemePreference } from '$lib/api/app';

export type ThemeMode = 'system' | 'dark' | 'light';

function applyTheme(mode: ThemeMode) {
  const root = document.documentElement;
  root.dataset.theme = mode;
}

function createThemeStore() {
  const { subscribe, set } = writable<ThemeMode>('system');

  return {
    subscribe,
    async init() {
      const mode = await getThemePreference().catch(() => 'system' as ThemeMode);
      set(mode);
      applyTheme(mode);
    },
    async set(mode: ThemeMode) {
      set(mode);
      applyTheme(mode);
      const savedMode = await setThemePreference(mode).catch(() => mode);
      set(savedMode);
      applyTheme(savedMode);
    }
  };
}

export const themeStore = createThemeStore();
