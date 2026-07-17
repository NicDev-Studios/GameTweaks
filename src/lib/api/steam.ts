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
  managedByGameTweaks: boolean;
}

export interface LocalizedText {
  en: string;
  de?: string;
}

export type AgentConnectionStatus =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'incompatible'
  | 'ambiguous';
export type GameSupportStatus = 'supported' | 'unsupported' | 'unavailable';
export type GameModStatus =
  | 'notInstalled'
  | 'installed'
  | 'updateAvailable'
  | 'blocked'
  | 'external';
export type ConfigApplyMode = 'live' | 'restartRequired' | 'nextLaunch';

interface ConfigFieldBase {
  id: string;
  section: string;
  key: string;
  label: LocalizedText;
  description?: LocalizedText;
  applyMode: ConfigApplyMode;
  locked: boolean;
}

export type ConfigField =
  | (ConfigFieldBase & {
      control: 'boolean';
      default: boolean;
      display: 'switch' | 'checkbox';
    })
  | (ConfigFieldBase & {
      control: 'string';
      default: string;
      maxLength: number;
    })
  | (ConfigFieldBase & {
      control: 'integer' | 'decimal';
      default: number;
      min: number;
      max: number;
      step: number;
    })
  | (ConfigFieldBase & {
      control: 'singleSelect';
      default: string;
      options: Array<{ value: string; label: LocalizedText }>;
      display: 'dropdown' | 'radio';
    })
  | (ConfigFieldBase & {
      control: 'multiSelect';
      default: string[];
      options: Array<{ value: string; label: LocalizedText }>;
    });

export interface GameMod {
  modId: string;
  guid: string;
  version: string;
  installedVersion?: string;
  official: boolean;
  external: boolean;
  name: LocalizedText;
  description: LocalizedText;
  integration: 'agent' | 'configFile';
  status: GameModStatus;
  dependencies: Array<{ modId: string; minimumVersion: string }>;
  conflicts: string[];
  config: ConfigField[];
  values: Record<string, unknown>;
  restartRequired: boolean;
}

export interface GameSupport {
  appId: number;
  status: GameSupportStatus;
  name?: LocalizedText;
  mods: GameMod[];
  agentInstalled: boolean;
  agentVersion?: string;
  agentStatus: AgentConnectionStatus;
  cached: boolean;
}

export interface ModActionPlan {
  planId: string;
  appId: number;
  modIds: string[];
  installsAgent: boolean;
  action: 'install' | 'update' | 'uninstall';
}

export interface ModInstallProgress {
  appId: number;
  modId: string;
  stage: 'downloading' | 'verifying' | 'installing' | 'completed';
  downloadedBytes: number;
  totalBytes?: number;
  percentage?: number;
}

export interface BepInExUninstallPlan {
  planId: string;
  appId: number;
  version: string;
  additionalFileCount: number;
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
const AGENT_STATE_EVENT = 'gametweaks-agent-state';
const AGENT_CONFIG_EVENT = 'gametweaks-agent-config-changed';
const MOD_INSTALL_PROGRESS_EVENT = 'gametweaks-mod-install-progress';

export interface AgentStateEvent {
  appId: number;
  status: AgentConnectionStatus;
}

export interface AgentConfigEvent {
  appId: number;
  modId: string;
  values: Record<string, unknown>;
}

export function listSteamGames(): Promise<SteamGame[]> {
  return invoke<SteamGame[]>('list_steam_games');
}

export function prepareBepInExInstall(appId: number): Promise<BepInExInstallPlan> {
  return invoke<BepInExInstallPlan>('prepare_bepinex_install', { appId });
}

export function getGameSupport(appId: number): Promise<GameSupport> {
  return invoke<GameSupport>('get_game_support', { appId });
}

export function installDevelopmentAgent(appId: number): Promise<GameSupport> {
  return invoke<GameSupport>('install_development_agent', { appId });
}

export function prepareBepInExUninstall(appId: number): Promise<BepInExUninstallPlan> {
  return invoke<BepInExUninstallPlan>('prepare_bepinex_uninstall', { appId });
}

export function uninstallBepInEx(planId: string): Promise<void> {
  return invoke<void>('uninstall_bepinex', { planId });
}

export function prepareModInstall(appId: number, modIds: string[]): Promise<ModActionPlan> {
  return invoke<ModActionPlan>('prepare_mod_install', { appId, modIds });
}

export function prepareModUpdate(appId: number, modId: string): Promise<ModActionPlan> {
  return invoke<ModActionPlan>('prepare_mod_update', { appId, modId });
}

export function installMods(planId: string): Promise<GameSupport> {
  return invoke<GameSupport>('install_mods', { planId });
}

export function updateMod(planId: string): Promise<GameSupport> {
  return invoke<GameSupport>('update_mod', { planId });
}

export function prepareModUninstall(
  appId: number,
  modId: string,
  removeConfig: boolean
): Promise<ModActionPlan> {
  return invoke<ModActionPlan>('prepare_mod_uninstall', { appId, modId, removeConfig });
}

export function uninstallMod(planId: string): Promise<GameSupport> {
  return invoke<GameSupport>('uninstall_mod', { planId });
}

export function setModConfig(
  appId: number,
  modId: string,
  changes: Record<string, unknown>
): Promise<GameSupport> {
  return invoke<GameSupport>('set_mod_config', { appId, modId, changes });
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

export async function listenToAgentEvents(
  onState: (event: AgentStateEvent) => void,
  onConfig: (event: AgentConfigEvent) => void
): Promise<() => void> {
  const unlistenState = await listen<AgentStateEvent>(AGENT_STATE_EVENT, (event) => {
    onState(event.payload);
  });
  try {
    const unlistenConfig = await listen<AgentConfigEvent>(AGENT_CONFIG_EVENT, (event) => {
      onConfig(event.payload);
    });
    return () => {
      unlistenConfig();
      unlistenState();
    };
  } catch (error) {
    unlistenState();
    throw error;
  }
}

export async function listenToModProgress(
  onProgress: (event: ModInstallProgress) => void
): Promise<() => void> {
  return listen<ModInstallProgress>(MOD_INSTALL_PROGRESS_EVENT, (event) => {
    onProgress(event.payload);
  });
}
