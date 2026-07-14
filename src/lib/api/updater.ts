import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { relaunch } from '@tauri-apps/plugin-process';

export type UpdateStatus =
  | 'idle'
  | 'checking'
  | 'available'
  | 'upToDate'
  | 'downloading'
  | 'readyToRestart'
  | 'error';

export interface UpdateInfo {
  currentVersion: string;
  version: string;
  date?: string;
  body?: string;
  channel: UpdateChannel;
}

export interface DownloadProgress {
  downloadedBytes: number;
  totalBytes?: number;
  percentage?: number;
}

export type UpdateChannel = 'stable' | 'beta';

const UPDATE_PROGRESS_EVENT = 'gametweaks-update-progress';

export async function checkForUpdate(): Promise<UpdateInfo | null> {
  return invoke<UpdateInfo | null>('check_for_update');
}

export async function downloadAndInstallUpdate(
  onProgress: (progress: DownloadProgress) => void
): Promise<void> {
  const unlisten = await listen<DownloadProgress>(UPDATE_PROGRESS_EVENT, (event) => {
    onProgress(event.payload);
  });

  try {
    await invoke('download_and_install_update');
  } finally {
    unlisten();
  }
}

export function restartApplication(): Promise<void> {
  return relaunch();
}

export function getUpdateChannel(): Promise<UpdateChannel> {
  return invoke<UpdateChannel>('get_update_channel');
}

export function setUpdateChannel(channel: UpdateChannel): Promise<UpdateChannel> {
  return invoke<UpdateChannel>('set_update_channel', { channel });
}
