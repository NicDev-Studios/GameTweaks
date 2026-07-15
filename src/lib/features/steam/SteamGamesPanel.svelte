<script lang="ts">
  import { onMount, tick } from 'svelte';
  import SectionHeader from '$lib/components/SectionHeader.svelte';
  import {
    installBepInEx,
    listSteamGames,
    prepareBepInExInstall,
    type BepInExArchitecture,
    type BepInExInstallPlan,
    type BepInExInstallProgress,
    type BepInExReason,
    type BepInExRuntime,
    type SteamGame
  } from '$lib/api/steam';
  import { t } from '$lib/i18n';

  type LoadState = 'loading' | 'ready' | 'error';

  let games: SteamGame[] = [];
  let loadState: LoadState = 'loading';
  let preparing: Record<number, boolean> = {};
  let installing: Record<number, boolean> = {};
  let progress: Record<number, BepInExInstallProgress> = {};
  let rowErrors: Record<number, string> = {};
  let pendingPlan: BepInExInstallPlan | undefined;
  let pendingGameName = '';
  let confirmationDialog: { showModal: () => void; close: () => void };

  onMount(() => {
    let mounted = true;

    listSteamGames()
      .then((installedGames) => {
        if (!mounted) return;
        games = installedGames;
        loadState = 'ready';
      })
      .catch(() => {
        if (!mounted) return;
        games = [];
        loadState = 'error';
      });

    return () => {
      mounted = false;
    };
  });

  function setPreparing(appId: number, active: boolean) {
    preparing = { ...preparing, [appId]: active };
  }

  function setInstalling(appId: number, active: boolean) {
    installing = { ...installing, [appId]: active };
  }

  function setRowError(appId: number, message = '') {
    rowErrors = { ...rowErrors, [appId]: message };
  }

  async function handlePrepare(game: SteamGame) {
    setPreparing(game.appId, true);
    setRowError(game.appId);
    try {
      pendingPlan = await prepareBepInExInstall(game.appId);
      pendingGameName = game.name;
      await tick();
      confirmationDialog.showModal();
    } catch (error) {
      setRowError(game.appId, installErrorMessage(error));
    } finally {
      setPreparing(game.appId, false);
    }
  }

  function closeConfirmation() {
    confirmationDialog.close();
    pendingPlan = undefined;
    pendingGameName = '';
  }

  async function handleInstall() {
    const plan = pendingPlan;
    if (!plan) return;

    confirmationDialog.close();
    pendingPlan = undefined;
    pendingGameName = '';
    setInstalling(plan.appId, true);
    setRowError(plan.appId);
    progress = {
      ...progress,
      [plan.appId]: {
        appId: plan.appId,
        stage: 'downloading',
        downloadedBytes: 0
      }
    };

    try {
      const result = await installBepInEx(plan.planId, (nextProgress) => {
        if (nextProgress.appId !== plan.appId) return;
        progress = { ...progress, [plan.appId]: nextProgress };
      });
      games = games.map((game) =>
        game.appId === result.appId
          ? {
              ...game,
              bepInEx: {
                status: 'installed',
                runtime: result.runtime,
                architecture: result.architecture,
                installedVersion: result.version
              }
            }
          : game
      );
    } catch (error) {
      setRowError(plan.appId, installErrorMessage(error));
    } finally {
      setInstalling(plan.appId, false);
      const remainingProgress = { ...progress };
      delete remainingProgress[plan.appId];
      progress = remainingProgress;
    }
  }

  function installErrorMessage(error: unknown): string {
    const code =
      typeof error === 'object' && error !== null && 'code' in error
        ? String((error as { code: unknown }).code)
        : '';
    const knownCodes = new Set([
      'bepinex_already_installed',
      'bepinex_blocked',
      'bepinex_busy',
      'bepinex_game_running',
      'bepinex_install_error',
      'bepinex_integrity_error',
      'bepinex_network_error',
      'bepinex_plan_expired',
      'bepinex_unsupported'
    ]);
    return $t(
      knownCodes.has(code)
        ? `steamGames.bepInEx.errors.${code}`
        : 'steamGames.bepInEx.errors.generic'
    );
  }

  function runtimeLabel(runtime?: BepInExRuntime): string {
    return runtime ? $t(`steamGames.bepInEx.runtime.${runtime}`) : '';
  }

  function architectureLabel(architecture?: BepInExArchitecture): string {
    return architecture?.toUpperCase() ?? '';
  }

  function reasonLabel(reason?: BepInExReason): string {
    return $t(`steamGames.bepInEx.reasons.${reason ?? 'inspectionFailed'}`);
  }

  function progressLabel(installProgress?: BepInExInstallProgress): string {
    if (!installProgress) return $t('steamGames.bepInEx.installing');
    if (installProgress.stage === 'downloading' && installProgress.percentage !== undefined) {
      return $t('steamGames.bepInEx.downloadingPercent', {
        percentage: installProgress.percentage
      });
    }
    return $t(`steamGames.bepInEx.progress.${installProgress.stage}`);
  }
</script>

<section class="single-panel glass-panel" aria-busy={loadState === 'loading'}>
  <SectionHeader
    eyebrow={$t('steamGames.eyebrow')}
    title={$t('steamGames.title')}
    description={$t('steamGames.description')}
  />

  {#if loadState === 'loading'}
    <p class="game-list-status" role="status">{$t('steamGames.loading')}</p>
  {:else if loadState === 'error'}
    <p class="game-list-status error" role="alert">{$t('steamGames.error')}</p>
  {:else if games.length === 0}
    <p class="game-list-status" role="status">{$t('steamGames.empty')}</p>
  {:else}
    <ul class="game-list" aria-label={$t('steamGames.listLabel')}>
      {#each games as game (game.appId)}
        <li class="game-row">
          <div class="game-details">
            <strong>{game.name}</strong>
            {#if game.bepInEx.runtime && game.bepInEx.architecture}
              <span class="game-runtime">
                {runtimeLabel(game.bepInEx.runtime)} · {architectureLabel(game.bepInEx.architecture)}
              </span>
            {/if}
            {#if game.bepInEx.status === 'unsupported' || game.bepInEx.status === 'blocked'}
              <span class:blocked={game.bepInEx.status === 'blocked'} class="game-compatibility">
                {reasonLabel(game.bepInEx.reason)}
              </span>
            {/if}
            {#if rowErrors[game.appId]}
              <span class="game-install-error" role="alert">{rowErrors[game.appId]}</span>
            {/if}
          </div>

          <div class="game-install-action">
            {#if game.bepInEx.status === 'installable'}
              <button
                class="primary-action game-install-button"
                type="button"
                disabled={preparing[game.appId] || installing[game.appId]}
                aria-busy={preparing[game.appId] || installing[game.appId]}
                on:click={() => handlePrepare(game)}
              >
                {#if preparing[game.appId] || installing[game.appId]}
                  <span class="button-spinner" aria-hidden="true"></span>
                {:else}
                  <span class="material-symbols-rounded" aria-hidden="true">download</span>
                {/if}
                <span>
                  {#if preparing[game.appId]}
                    {$t('steamGames.bepInEx.checking')}
                  {:else if installing[game.appId]}
                    {progressLabel(progress[game.appId])}
                  {:else}
                    {$t('steamGames.bepInEx.install')}
                  {/if}
                </span>
              </button>
            {:else if game.bepInEx.status === 'installed'}
              <span class="game-installed-status">
                <span class="material-symbols-rounded" aria-hidden="true">check_circle</span>
                <span>
                  {#if game.bepInEx.installedVersion}
                    {$t('steamGames.bepInEx.installedVersion', {
                      version: game.bepInEx.installedVersion
                    })}
                  {:else}
                    {$t('steamGames.bepInEx.installed')}
                  {/if}
                </span>
              </span>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<dialog
  class="bepinex-confirmation glass-panel"
  bind:this={confirmationDialog}
  aria-labelledby="bepinex-confirmation-title"
  on:cancel={(event) => {
    event.preventDefault();
    closeConfirmation();
  }}
>
  {#if pendingPlan}
    <div class="bepinex-confirmation-content">
      <div>
        <p class="eyebrow">{$t('steamGames.bepInEx.confirmEyebrow')}</p>
        <h2 id="bepinex-confirmation-title">{$t('steamGames.bepInEx.confirmTitle')}</h2>
      </div>
      <p>
        {$t('steamGames.bepInEx.confirmDescription', {
          game: pendingGameName,
          version: pendingPlan.version,
          runtime: runtimeLabel(pendingPlan.runtime),
          architecture: architectureLabel(pendingPlan.architecture)
        })}
      </p>
      <p class="bepinex-confirmation-warning">
        <span class="material-symbols-rounded" aria-hidden="true">warning</span>
        <span>{$t('steamGames.bepInEx.closeGameWarning')}</span>
      </p>
      <div class="bepinex-confirmation-actions">
        <button class="secondary-action" type="button" on:click={closeConfirmation}>
          {$t('steamGames.bepInEx.cancel')}
        </button>
        <button class="primary-action" type="button" on:click={handleInstall}>
          <span class="material-symbols-rounded" aria-hidden="true">download</span>
          <span>{$t('steamGames.bepInEx.confirmInstall')}</span>
        </button>
      </div>
    </div>
  {/if}
</dialog>
