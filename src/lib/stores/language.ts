import { get, writable } from 'svelte/store';
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

  return {
    subscribe,
    async init() {
      const mode = await getLanguagePreference().catch(() => 'automatic' as LanguageMode);
      set(mode);
      applyLanguage(mode);
    },
    async set(mode: LanguageMode) {
      set(mode);
      applyLanguage(mode);
      const savedMode = await setLanguagePreference(mode).catch(() => mode);
      set(savedMode);
      applyLanguage(savedMode);
    },
    refreshSystemLanguage() {
      const mode = get(languageStoreInternal);
      applyLanguage(mode);
    }
  };
}

export const languageStore = createLanguageStore();
