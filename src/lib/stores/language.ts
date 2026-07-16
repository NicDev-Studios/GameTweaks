import { writable } from 'svelte/store';
import { getLanguagePreference, setLanguagePreference } from '$lib/api/app';

export type SupportedLanguage = 'en' | 'de';
export type LanguageMode = 'automatic' | SupportedLanguage;

const supportedLanguages: SupportedLanguage[] = ['en', 'de'];
const languageStoreInternal = writable<LanguageMode>('automatic');
export const activeLanguageStore = writable<SupportedLanguage>('en');

function detectSystemLanguage(): SupportedLanguage {
  const languages = globalThis.navigator?.languages?.length
    ? globalThis.navigator.languages
    : [globalThis.navigator?.language ?? 'en'];

  for (const language of languages) {
    const baseLanguage = language.toLowerCase().split('-')[0];
    if (supportedLanguages.includes(baseLanguage as SupportedLanguage)) {
      return baseLanguage as SupportedLanguage;
    }
  }

  return 'en';
}

function resolveLanguage(mode: LanguageMode): SupportedLanguage {
  return mode === 'automatic' ? detectSystemLanguage() : mode;
}

function applyLanguage(mode: LanguageMode) {
  const activeLanguage = resolveLanguage(mode);
  activeLanguageStore.set(activeLanguage);

  if (globalThis.document) {
    document.documentElement.lang = activeLanguage;
  }
}

function createLanguageStore() {
  const { subscribe, set } = languageStoreInternal;
  let persistedMode: LanguageMode = 'automatic';
  let revision = 0;
  let writeQueue = Promise.resolve();

  return {
    subscribe,
    async init() {
      const initRevision = revision;
      const mode = await getLanguagePreference().catch(() => 'automatic' as LanguageMode);
      if (initRevision !== revision) return;

      persistedMode = mode;
      set(mode);
      applyLanguage(mode);
    },
    async set(mode: LanguageMode) {
      const requestRevision = ++revision;
      set(mode);
      applyLanguage(mode);

      const write = writeQueue.then(() => setLanguagePreference(mode));
      writeQueue = write.then(
        () => undefined,
        () => undefined
      );

      try {
        const savedMode = await write;
        persistedMode = savedMode;
        if (requestRevision !== revision) return;

        set(savedMode);
        applyLanguage(savedMode);
      } catch {
        if (requestRevision !== revision) return;

        set(persistedMode);
        applyLanguage(persistedMode);
      }
    }
  };
}

export const languageStore = createLanguageStore();
