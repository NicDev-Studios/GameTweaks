import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type BepInExRuntime = 'mono' | 'il2Cpp';
export type BepInExArchitecture = 'x86' | 'x64';
export type BepInExStatus = 'installable' | 'installed' | 'unsupported' | 'blocked';
export type BepInExReason =
  | 'windowsOnly'
  | 'notUnity'
  | 'ambiguousExecutable'
  | 'ambiguousRuntime'
  | 'unsupportedArchitecture'
  | 'inspectionFailed'
  | 'unsafeSymlink'
  | 'antiCheatDetected'
  | 'existingFiles';

export interface BepInExGameStatus {
  status: BepInExStatus;
  runtime?: BepInExRuntime;
  architecture?: BepInExArchitecture;
  installedVersion?: string;
  reason?: BepInExReason;
}

export interface SteamGame {
  appId: number;
  name: string;
  bepInEx: BepInExGameStatus;
}

export interface BepInExInstallPlan {
  planId: string;
  appId: number;
  version: string;
  runtime: BepInExRuntime;
  architecture: BepInExArchitecture;
  releaseChannel: 'stable' | 'bleedingEdge';
}

export interface BepInExInstallResult {
  appId: number;
  version: string;
  runtime: BepInExRuntime;
  architecture: BepInExArchitecture;
}

export interface BepInExInstallProgress {
  appId: number;
  stage: 'downloading' | 'verifying' | 'installing' | 'completed';
  downloadedBytes: number;
  totalBytes?: number;
  percentage?: number;
}

const BEPINEX_INSTALL_PROGRESS_EVENT = 'gametweaks-bepinex-install-progress';

export function listSteamGames(): Promise<SteamGame[]> {
  return invoke<SteamGame[]>('list_steam_games');
}

export function prepareBepInExInstall(appId: number): Promise<BepInExInstallPlan> {
  return invoke<BepInExInstallPlan>('prepare_bepinex_install', { appId });
}

export async function installBepInEx(
  planId: string,
  onProgress: (progress: BepInExInstallProgress) => void
): Promise<BepInExInstallResult> {
  const unlisten = await listen<BepInExInstallProgress>(BEPINEX_INSTALL_PROGRESS_EVENT, (event) => {
    onProgress(event.payload);
  });

  try {
    return await invoke<BepInExInstallResult>('install_bepinex', { planId });
  } finally {
    unlisten();
  }
}
