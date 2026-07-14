import { invoke } from '@tauri-apps/api/core';

export interface SteamGame {
  appId: number;
  name: string;
}

export function listSteamGames(): Promise<SteamGame[]> {
  return invoke<SteamGame[]>('list_steam_games');
}
