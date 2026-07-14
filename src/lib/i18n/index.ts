import { derived } from 'svelte/store';
import { activeLanguageStore, type SupportedLanguage } from '$lib/stores/language';
import deMessages from './locales/de/common.json';
import enMessages from './locales/en/common.json';

type TranslationValue = string | { [key: string]: TranslationValue };
type TranslationDictionary = Record<string, TranslationValue>;
type TranslateParams = Record<string, string | number>;
type TranslateFunction = (key: string, params?: TranslateParams) => string;

const messages: Record<SupportedLanguage, TranslationDictionary> = {
  en: enMessages,
  de: deMessages
};

function lookup(dictionary: TranslationDictionary, key: string): string | undefined {
  const value = key.split('.').reduce<TranslationValue | undefined>((current, part) => {
    if (!current || typeof current === 'string') return undefined;
    return current[part];
  }, dictionary);

  return typeof value === 'string' ? value : undefined;
}

function interpolate(template: string, params: TranslateParams = {}): string {
  return template.replace(/\{([a-zA-Z0-9_]+)\}/g, (match, name: string) => {
    const value = params[name];
    return value === undefined ? match : String(value);
  });
}

export const t = derived<typeof activeLanguageStore, TranslateFunction>(
  activeLanguageStore,
  ($activeLanguage) =>
    (key: string, params?: TranslateParams) => {
      const template = lookup(messages[$activeLanguage], key) ?? lookup(messages.en, key) ?? key;
      return interpolate(template, params);
    }
);
