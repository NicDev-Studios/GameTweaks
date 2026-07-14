import { invoke } from '@tauri-apps/api/core';
import type { LanguageMode } from '$lib/stores/language';
import type { ThemeMode } from '$lib/stores/theme';

export interface AppOverview {
  name: string;
  version: string;
  configVersion: number;
}

export function getAppOverview(): Promise<AppOverview> {
  return invoke<AppOverview>('get_app_overview');
}

export function getThemePreference(): Promise<ThemeMode> {
  return invoke<ThemeMode>('get_theme_preference');
}

export function setThemePreference(theme: ThemeMode): Promise<ThemeMode> {
  return invoke<ThemeMode>('set_theme_preference', { theme });
}

export function getLanguagePreference(): Promise<LanguageMode> {
  return invoke<LanguageMode>('get_language_preference');
}

export function setLanguagePreference(language: LanguageMode): Promise<LanguageMode> {
  return invoke<LanguageMode>('set_language_preference', { language });
}
