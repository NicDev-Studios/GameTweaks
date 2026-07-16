import { writable } from 'svelte/store';
import {
  getDeveloperMode,
  setDeveloperMode,
  type DeveloperModeState
} from '$lib/api/app';

const fallback: DeveloperModeState = {
  enabled: import.meta.env.DEV,
  forced: import.meta.env.DEV
};

function createDeveloperModeStore() {
  const { subscribe, set } = writable<DeveloperModeState>(fallback);

  return {
    subscribe,
    async init() {
      set(await getDeveloperMode().catch(() => fallback));
    },
    async setEnabled(enabled: boolean) {
      const state = await setDeveloperMode(enabled).catch(() =>
        getDeveloperMode().catch(() => fallback)
      );
      set(state);
    }
  };
}

export const developerModeStore = createDeveloperModeStore();
