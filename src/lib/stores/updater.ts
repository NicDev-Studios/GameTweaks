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
  checkedOnStartup: boolean;
  displayStatus: UpdateStatus;
  error: string;
  errorDetail: string;
  info?: UpdateInfo;
  progress: DownloadProgress;
  status: UpdateStatus;
}

const initialState: UpdaterState = {
  channel: 'stable',
  checkedOnStartup: false,
  displayStatus: 'idle',
  error: '',
  errorDetail: '',
  progress: { downloadedBytes: 0 },
  status: 'idle'
};

function createUpdaterStore() {
  const { subscribe, update } = writable<UpdaterState>(initialState);
  let initializePromise: Promise<void> | undefined;
  let startupCheckPromise: Promise<void> | undefined;

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

  const checkForUpdates = async (checkedOnStartup = false) => {
    const startedAt = Date.now();

    update((state) => ({
      ...state,
      checkedOnStartup: state.checkedOnStartup || checkedOnStartup,
      status: 'checking'
    }));

    try {
      const info = await checkForUpdate();
      await waitForMinimumDuration(startedAt, 500);

      const status = info ? 'available' : 'upToDate';
      update((state) => ({
        ...state,
        checkedOnStartup: state.checkedOnStartup || checkedOnStartup,
        error: '',
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
        checkedOnStartup: state.checkedOnStartup || checkedOnStartup,
        displayStatus: 'error',
        error: getUserFacingErrorMessage(error),
        errorDetail: getErrorMessage(error),
        status: 'error'
      }));
    }
  };

  return {
    subscribe,
    checkForUpdates,
    initialize,
    checkForUpdatesOnStartup() {
      if (!startupCheckPromise) {
        startupCheckPromise = initialize().then(() => checkForUpdates(true));
      }

      return startupCheckPromise;
    },
    async downloadAndInstall() {
      update((state) => ({
        ...state,
        error: '',
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
          error: getUserFacingErrorMessage(error),
          errorDetail: getErrorMessage(error),
          status: 'error'
        }));
      }
    },
    async restart() {
      update((state) => ({ ...state, error: '', errorDetail: '' }));

      try {
        await restartApplication();
      } catch (error) {
        reportUpdaterError(error);
        update((state) => ({
          ...state,
          displayStatus: 'error',
          error: getUserFacingErrorMessage(error),
          errorDetail: getErrorMessage(error),
          status: 'error'
        }));
      }
    },
    async setChannel(channel: UpdateChannel) {
      const savedChannel = await setUpdateChannel(channel);
      startupCheckPromise = undefined;
      update((state) => ({
        ...state,
        channel: savedChannel,
        displayStatus: 'idle',
        error: '',
        errorDetail: '',
        info: undefined,
        progress: { downloadedBytes: 0 },
        status: 'idle'
      }));
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

function getUserFacingErrorMessage(error: unknown) {
  const message = getErrorMessage(error);

  if (message.toLowerCase().includes('release json')) {
    return 'Update check failed because the release metadata is not available.';
  }

  return 'Update check failed. Try again later.';
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
