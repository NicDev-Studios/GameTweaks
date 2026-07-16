import { writable } from 'svelte/store';
import {
  checkForUpdate,
  downloadAndInstallUpdate,
  getUpdateChannel,
  restartApplication,
  setUpdateChannel,
  type DownloadProgress,
  type UpdateChannel,
  type UpdateInfo,
  type UpdateStatus
} from '$lib/api/updater';

export interface UpdaterState {
  channel: UpdateChannel;
  channelChanging: boolean;
  displayStatus: UpdateStatus;
  error?: UpdaterError;
  errorDetail: string;
  info?: UpdateInfo;
  progress: DownloadProgress;
  status: UpdateStatus;
}

export type UpdaterError = 'check' | 'checkMetadata' | 'download' | 'restart' | 'channel';

const initialState: UpdaterState = {
  channel: 'stable',
  channelChanging: false,
  displayStatus: 'idle',
  errorDetail: '',
  progress: { downloadedBytes: 0 },
  status: 'idle'
};

function createUpdaterStore() {
  const { subscribe, update } = writable<UpdaterState>(initialState);
  let initializePromise: Promise<void> | undefined;
  let startupCheckPromise: Promise<void> | undefined;
  let channelChangePromise: Promise<void> | undefined;

  const initialize = async () => {
    if (!initializePromise) {
      initializePromise = getUpdateChannel()
        .then((channel) => {
          update((state) => ({ ...state, channel }));
        })
        .catch(reportUpdaterError);
    }

    return initializePromise;
  };

  const checkForUpdates = async () => {
    const startedAt = Date.now();

    update((state) => ({
      ...state,
      error: undefined,
      errorDetail: '',
      status: 'checking'
    }));

    try {
      const info = await checkForUpdate();
      await waitForMinimumDuration(startedAt, 500);

      const status = info ? 'available' : 'upToDate';
      update((state) => ({
        ...state,
        error: undefined,
        errorDetail: '',
        displayStatus: status,
        info: info ?? undefined,
        progress: { downloadedBytes: 0 },
        status
      }));
    } catch (error) {
      await waitForMinimumDuration(startedAt, 500);
      reportUpdaterError(error);
      update((state) => ({
        ...state,
        displayStatus: 'error',
        error: getUpdaterError(error, 'check'),
        errorDetail: getErrorMessage(error),
        status: 'error'
      }));
    }
  };

  const changeChannel = async (channel: UpdateChannel) => {
    update((state) => ({ ...state, channelChanging: true }));
    try {
      const savedChannel = await setUpdateChannel(channel);
      startupCheckPromise = undefined;
      update((state) => ({
        ...state,
        channel: savedChannel,
        displayStatus: 'idle',
        error: undefined,
        errorDetail: '',
        info: undefined,
        progress: { downloadedBytes: 0 },
        status: 'idle'
      }));
    } catch (error) {
      reportUpdaterError(error);
      update((state) => ({
        ...state,
        displayStatus: 'error',
        error: 'channel',
        errorDetail: getErrorMessage(error),
        status: 'error'
      }));
    } finally {
      update((state) => ({ ...state, channelChanging: false }));
    }
  };

  return {
    subscribe,
    checkForUpdates,
    initialize,
    checkForUpdatesOnStartup() {
      if (!startupCheckPromise) {
        startupCheckPromise = initialize().then(checkForUpdates);
      }

      return startupCheckPromise;
    },
    async downloadAndInstall() {
      update((state) => ({
        ...state,
        error: undefined,
        errorDetail: '',
        progress: { downloadedBytes: 0 },
        status: 'downloading'
      }));

      try {
        await downloadAndInstallUpdate((progress) => {
          update((state) => ({ ...state, progress }));
        });
        update((state) => ({
          ...state,
          displayStatus: 'readyToRestart',
          status: 'readyToRestart'
        }));
      } catch (error) {
        reportUpdaterError(error);
        update((state) => ({
          ...state,
          displayStatus: 'error',
          error: 'download',
          errorDetail: getErrorMessage(error),
          status: 'error'
        }));
      }
    },
    async restart() {
      update((state) => ({ ...state, error: undefined, errorDetail: '' }));

      try {
        await restartApplication();
      } catch (error) {
        reportUpdaterError(error);
        update((state) => ({
          ...state,
          displayStatus: 'error',
          error: 'restart',
          errorDetail: getErrorMessage(error),
          status: 'error'
        }));
      }
    },
    setChannel(channel: UpdateChannel) {
      if (!channelChangePromise) {
        channelChangePromise = changeChannel(channel).finally(() => {
          channelChangePromise = undefined;
        });
      }

      return channelChangePromise;
    }
  };
}

function getErrorMessage(error: unknown) {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  if (isErrorResponse(error)) return error.message;

  return 'Update operation failed.';
}

function isErrorResponse(error: unknown): error is { message: string } {
  return (
    typeof error === 'object' &&
    error !== null &&
    'message' in error &&
    typeof error.message === 'string'
  );
}

function getUpdaterError(error: unknown, fallback: UpdaterError): UpdaterError {
  const message = getErrorMessage(error);

  if (message.toLowerCase().includes('release json')) {
    return 'checkMetadata';
  }

  return fallback;
}

function reportUpdaterError(error: unknown) {
  if (import.meta.env.DEV) {
    console.error('[GameTweaks updater]', getErrorMessage(error), error);
  }
}

function waitForMinimumDuration(startedAt: number, minimumDurationMs: number) {
  const remaining = minimumDurationMs - (Date.now() - startedAt);

  if (remaining <= 0) {
    return Promise.resolve();
  }

  return new Promise((resolve) => {
    setTimeout(resolve, remaining);
  });
}

export const updaterStore = createUpdaterStore();
