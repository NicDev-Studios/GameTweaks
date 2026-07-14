<script lang="ts">
  import SectionHeader from '$lib/components/SectionHeader.svelte';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { getAppOverview, type AppOverview } from '$lib/api/app';
  import { t } from '$lib/i18n';
  import { languageStore, type LanguageMode } from '$lib/stores/language';
  import { themeStore, type ThemeMode } from '$lib/stores/theme';
  import { updaterStore } from '$lib/stores/updater';
  import type { UpdateStatus } from '$lib/api/updater';

  const modes: Array<{ id: ThemeMode; labelKey: string }> = [
    { id: 'system', labelKey: 'appearance.system' },
    { id: 'dark', labelKey: 'appearance.dark' },
    { id: 'light', labelKey: 'appearance.light' }
  ];

  const languageModes: Array<{ id: LanguageMode; labelKey: string }> = [
    { id: 'automatic', labelKey: 'settings.language.automatic' },
    { id: 'en', labelKey: 'settings.language.english' }
  ];

  let overview: AppOverview | undefined;
  $: updateIcon =
    $updaterStore.displayStatus === 'available'
      ? 'system_update_alt'
      : $updaterStore.displayStatus === 'readyToRestart'
        ? 'restart_alt'
        : $updaterStore.displayStatus === 'error'
          ? 'error'
          : 'verified';
  $: updateBusy = $updaterStore.status === 'checking' || $updaterStore.status === 'downloading';
  $: updateChecking = $updaterStore.status === 'checking';
  $: updateFailed = $updaterStore.status === 'error';
  $: showManualUpdateCheck =
    $updaterStore.status !== 'available' &&
    $updaterStore.status !== 'readyToRestart' &&
    $updaterStore.status !== 'downloading';
  $: showUpdateError = updateFailed || (updateChecking && Boolean($updaterStore.error));
  $: updateMessage =
    updateChecking && $updaterStore.error
      ? $updaterStore.error
      : getUpdateMessage($updaterStore.displayStatus);

  getAppOverview()
    .then((appOverview) => {
      overview = appOverview;
    })
    .catch(() => {
      overview = undefined;
    });

  async function handleCheckForUpdate() {
    await updaterStore.checkForUpdates();
  }

  async function handleDownloadAndInstall() {
    await updaterStore.downloadAndInstall();
  }

  async function handleRestart() {
    await updaterStore.restart();
  }

  async function handleUpdateChannelChange(channel: 'stable' | 'beta') {
    if ($updaterStore.channel === channel || updateBusy) return;

    await updaterStore.setChannel(channel);
  }

  async function handleLanguageChange(event: { currentTarget: { value: string } }) {
    const value = event.currentTarget.value as LanguageMode;
    await languageStore.set(value);
  }

  async function handleOpenRepository() {
    await openUrl('https://github.com/NicDev-Studios/GameTweaks');
  }

  function getUpdateMessage(status: UpdateStatus) {
    if (status === 'available') {
      return $t('updates.available', { version: $updaterStore.info?.version ?? '' });
    }
    if (status === 'upToDate') return $t('updates.upToDate');
    if (status === 'downloading') return $t('updates.downloading');
    if (status === 'readyToRestart') return $t('updates.readyToRestart');
    if (status === 'error') return $updaterStore.error || $t('updates.failed');

    return $t('updates.startupCheck');
  }
</script>

<section class="single-panel glass-panel">
  <SectionHeader
    eyebrow={$t('settings.eyebrow')}
    title={$t('settings.title')}
    description={$t('settings.description')}
  />

  <div class="settings-row update-card">
    <div class="update-summary">
      <span class="material-symbols-rounded update-icon" aria-hidden="true">{updateIcon}</span>
      <div>
        <h2>{$t('updates.title')}</h2>
        <p>{$t('updates.currentVersion', { version: overview?.version ?? $t('updates.unknownVersion') })}</p>
      </div>
    </div>

    <div class:error={showUpdateError} class="update-control">
      <div class="update-message-row">
        <p class:error={showUpdateError}>{updateMessage}</p>

        {#if showUpdateError && $updaterStore.errorDetail}
          <span class="update-error-tip">
            <button
              class="update-error-trigger material-symbols-rounded"
              type="button"
              aria-label={$t('updates.errorDetails')}
            >
              priority_high
            </button>
            <span class="update-error-tooltip" role="tooltip">{$updaterStore.errorDetail}</span>
          </span>
        {/if}
      </div>

      {#if $updaterStore.info?.body && $updaterStore.status === 'available'}
        <p class="update-notes">{$updaterStore.info.body}</p>
      {/if}

      {#if $updaterStore.status === 'downloading'}
        <div
          class="update-progress"
          role="progressbar"
          aria-label={$t('updates.downloadProgress')}
          aria-valuenow={$updaterStore.progress.percentage}
          aria-valuemin="0"
          aria-valuemax="100"
        >
          <span style={`width: ${$updaterStore.progress.percentage ?? 12}%`}></span>
        </div>
      {/if}

      <div class="update-toolbar">
        <div class="segmented-control update-channel-control" role="group" aria-label={$t('updates.channel')}>
          <button
            type="button"
            class:active={$updaterStore.channel === 'stable'}
            disabled={updateBusy}
            on:click={() => handleUpdateChannelChange('stable')}
          >
            {$t('updates.stable')}
          </button>
          <button
            type="button"
            class:active={$updaterStore.channel === 'beta'}
            disabled={updateBusy}
            on:click={() => handleUpdateChannelChange('beta')}
          >
            {$t('updates.beta')}
          </button>
        </div>

        <div class="update-actions">
          {#if showManualUpdateCheck}
            <button
              class="secondary-action update-check-action"
              class:checking={updateChecking}
              class:check-failed={updateFailed}
              type="button"
              disabled={updateBusy}
              aria-busy={updateChecking}
              on:click={handleCheckForUpdate}
            >
              <span class="update-check-icon" aria-hidden="true">
                {#if updateChecking}
                  <span class="button-spinner"></span>
                {:else}
                  <span class="material-symbols-rounded">sync</span>
                {/if}
              </span>
              <span>{$t('updates.check')}</span>
            </button>
          {/if}

          {#if $updaterStore.status === 'available'}
            <button class="primary-action update-primary-action" type="button" on:click={handleDownloadAndInstall}>
              <span class="material-symbols-rounded" aria-hidden="true">download</span>
              <span>{$t('updates.download')}</span>
            </button>
          {:else if $updaterStore.status === 'readyToRestart'}
            <button class="primary-action update-primary-action" type="button" on:click={handleRestart}>
              <span class="material-symbols-rounded" aria-hidden="true">restart_alt</span>
              <span>{$t('updates.restart')}</span>
            </button>
          {/if}
        </div>
      </div>
    </div>
  </div>

  <div class="settings-row">
    <div>
      <h2>{$t('settings.language.title')}</h2>
      <p>{$t('settings.language.description')}</p>
    </div>
    <label class="settings-select-control">
      <span class="sr-only">{$t('settings.language.ariaLabel')}</span>
      <select value={$languageStore} aria-label={$t('settings.language.ariaLabel')} on:change={handleLanguageChange}>
        {#each languageModes as mode (mode.id)}
          <option value={mode.id}>{$t(mode.labelKey)}</option>
        {/each}
      </select>
      <span class="material-symbols-rounded" aria-hidden="true">expand_more</span>
    </label>
  </div>

  <div class="settings-row">
    <div>
      <h2>{$t('appearance.title')}</h2>
      <p>{$t('appearance.themeMode')}</p>
    </div>
    <div class="segmented-control" role="group" aria-label={$t('appearance.themeModeLabel')}>
      {#each modes as mode (mode.id)}
        <button
          type="button"
          class:active={$themeStore === mode.id}
          on:click={() => themeStore.set(mode.id)}
        >
          {$t(mode.labelKey)}
        </button>
      {/each}
    </div>
  </div>

  <footer class="settings-footer">
    <span aria-hidden="true">❤️</span>
    <span>Made with love by NicDev Studios</span>
    <button
      type="button"
      aria-label="GameTweaks on GitHub"
      title="GameTweaks on GitHub"
      on:click={handleOpenRepository}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path
          fill="currentColor"
          d="M12 .7a11.5 11.5 0 0 0-3.64 22.4c.58.1.79-.25.79-.56v-2.23c-3.22.7-3.9-1.37-3.9-1.37-.53-1.34-1.29-1.7-1.29-1.7-1.05-.72.08-.71.08-.71 1.16.08 1.78 1.2 1.78 1.2 1.03 1.77 2.71 1.26 3.37.96.1-.75.4-1.26.73-1.55-2.57-.29-5.27-1.29-5.27-5.68 0-1.26.45-2.28 1.19-3.09-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.16 1.18a10.98 10.98 0 0 1 5.76 0c2.19-1.49 3.15-1.18 3.15-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.83 1.19 3.09 0 4.4-2.71 5.38-5.29 5.67.42.36.79 1.06.79 2.14v3.17c0 .31.21.67.8.56A11.5 11.5 0 0 0 12 .7Z"
        />
      </svg>
    </button>
  </footer>
</section>
