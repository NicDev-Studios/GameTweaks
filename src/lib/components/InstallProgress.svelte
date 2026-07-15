<script lang="ts">
  import type { BepInExInstallProgress } from '$lib/api/steam';
  import { t } from '$lib/i18n';
  import { activeLanguageStore } from '$lib/stores/language';

  export let progress: BepInExInstallProgress;

  $: percentage =
    progress.stage === 'completed'
      ? 100
      : progress.percentage === undefined
        ? undefined
        : Math.max(0, Math.min(100, progress.percentage));
  $: phase = $t(`steamGames.bepInEx.progress.${progress.stage}`);

  function formatBytes(bytes: number) {
    const units = ['B', 'KB', 'MB', 'GB'];
    let value = Math.max(0, bytes);
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
      value /= 1024;
      unit += 1;
    }
    return `${new Intl.NumberFormat($activeLanguageStore, {
      maximumFractionDigits: unit === 0 ? 0 : 1
    }).format(value)} ${units[unit]}`;
  }
</script>

<div class="install-progress" role="status" aria-live="polite">
  <div class="install-progress-heading">
    <span class="install-progress-phase">
      <span
        class:mod-spinner={progress.stage !== 'completed'}
        class="material-symbols-rounded"
        aria-hidden="true"
      >
        {progress.stage === 'completed' ? 'check_circle' : 'progress_activity'}
      </span>
      <span>{phase}</span>
    </span>
    {#if percentage !== undefined}<strong>{percentage}%</strong>{/if}
  </div>

  <div
    class:indeterminate={percentage === undefined}
    class="install-progress-track"
    role="progressbar"
    aria-label={phase}
    aria-valuemin="0"
    aria-valuemax="100"
    aria-valuenow={percentage}
  >
    <span style:width={percentage === undefined ? undefined : `${percentage}%`}></span>
  </div>

  {#if progress.stage === 'downloading' && progress.downloadedBytes > 0}
    <span class="install-progress-bytes">
      {formatBytes(progress.downloadedBytes)}{#if progress.totalBytes !== undefined}
        / {formatBytes(progress.totalBytes)}
      {/if}
    </span>
  {/if}
</div>
