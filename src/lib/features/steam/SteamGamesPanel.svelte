<script lang="ts">
  import { onMount, tick } from 'svelte';
  import InstallProgress from '$lib/components/InstallProgress.svelte';
  import SectionHeader from '$lib/components/SectionHeader.svelte';
  import {
    getGameSupport,
    installDevelopmentAgent,
    installBepInEx,
    installMods,
    listenToAgentEvents,
    listenToModProgress,
    listSteamGames,
    prepareBepInExInstall,
    prepareBepInExUninstall,
    prepareModInstall,
    prepareModUninstall,
    prepareModUpdate,
    setModConfig,
    uninstallBepInEx,
    uninstallMod,
    updateMod,
    type BepInExInstallPlan,
    type BepInExInstallProgress,
    type BepInExReason,
    type GameMod,
    type GameSupport,
    type LocalizedText,
    type ModActionPlan,
    type ModInstallProgress,
    type SteamGame
  } from '$lib/api/steam';
  import { t } from '$lib/i18n';
  import { activeLanguageStore } from '$lib/stores/language';
  import { developerModeStore } from '$lib/stores/developerMode';

  type LoadState = 'loading' | 'ready' | 'error';
  type PendingAction =
    | { kind: 'bepinexInstall'; plan: BepInExInstallPlan }
    | { kind: 'developmentAgent'; appId: number }
    | {
        kind: 'bepinexUninstall';
        plan: { planId: string; appId: number; version: string; additionalFileCount: number };
      }
    | { kind: 'mod'; plan: ModActionPlan; modId: string; modName: string; removeConfig?: boolean };

  let games: SteamGame[] = [];
  let loadState: LoadState = 'loading';
  let selectedGame: SteamGame | undefined;
  let support: GameSupport | undefined;
  let supportLoading = false;
  let actionBusy = false;
  let busyModId: string | undefined;
  let pageError = '';
  let modErrors: Record<string, string> = {};
  let pendingAction: PendingAction | undefined;
  let confirmationDialog: globalThis.HTMLDialogElement;
  let progress: BepInExInstallProgress | undefined;
  let removeConfigOnUninstall = false;
  let drafts: Record<string, Record<string, unknown>> = {};
  let dirtyMods = new Set<string>();
  let modProgress: Record<string, ModInstallProgress> = {};

  onMount(() => {
    void refreshGames();
    let disposed = false;
    let stopListening: (() => void) | undefined;
    let stopModProgress: (() => void) | undefined;
    void listenToAgentEvents(
      (event) => {
        if (event.appId === selectedGame?.appId) void refreshRuntimeSupport();
      },
      (event) => applyAgentValues(event.appId, event.modId, event.values)
    )
      .then((stop) => {
        if (disposed) stop();
        else stopListening = stop;
      })
      .catch(() => {
        // The detail view remains usable without optional live Agent events.
      });
    void listenToModProgress((event) => {
      if (event.appId !== selectedGame?.appId) return;
      modProgress = { ...modProgress, [event.modId]: event };
    })
      .then((stop) => {
        if (disposed) stop();
        else stopModProgress = stop;
      })
      .catch(() => {
        // Installation still works if the optional progress listener is unavailable.
      });
    return () => {
      disposed = true;
      stopListening?.();
      stopModProgress?.();
    };
  });

  async function refreshGames() {
    loadState = 'loading';
    try {
      games = await listSteamGames();
      if (selectedGame) {
        selectedGame = games.find((game) => game.appId === selectedGame?.appId);
      }
      loadState = 'ready';
    } catch {
      games = [];
      loadState = 'error';
    }
  }

  async function openGame(game: SteamGame) {
    selectedGame = game;
    support = undefined;
    pageError = '';
    modErrors = {};
    supportLoading = true;
    try {
      support = await getGameSupport(game.appId);
      resetDrafts();
    } catch (error) {
      pageError = errorMessage(error);
    } finally {
      supportLoading = false;
    }
  }

  function closeGame() {
    selectedGame = undefined;
    support = undefined;
    pageError = '';
    modErrors = {};
    dirtyMods = new Set();
    modProgress = {};
  }

  async function reloadDetail() {
    if (!selectedGame) return;
    await refreshGames();
    if (!selectedGame) return;
    const next = await getGameSupport(selectedGame.appId);
    support = next;
    const nextDrafts = { ...drafts };
    for (const mod of next.mods) {
      if (!dirtyMods.has(mod.modId)) nextDrafts[mod.modId] = { ...mod.values };
    }
    drafts = nextDrafts;
  }

  async function refreshRuntimeSupport() {
    if (!selectedGame) return;
    const appId = selectedGame.appId;
    try {
      const next = await getGameSupport(appId);
      if (selectedGame?.appId !== appId) return;
      support = next;
      const nextDrafts = { ...drafts };
      for (const mod of next.mods) {
        if (!dirtyMods.has(mod.modId)) nextDrafts[mod.modId] = { ...mod.values };
      }
      drafts = nextDrafts;
    } catch {
      // A transient catalog error must not discard the currently visible state.
    }
  }

  function applyAgentValues(appId: number, modId: string, values: Record<string, unknown>) {
    if (selectedGame?.appId !== appId || !support) return;
    support = {
      ...support,
      mods: support.mods.map((mod) =>
        mod.modId === modId ? { ...mod, values: { ...mod.values, ...values } } : mod
      )
    };
    if (!dirtyMods.has(modId)) {
      drafts = { ...drafts, [modId]: { ...(drafts[modId] ?? {}), ...values } };
    }
  }

  function resetDrafts() {
    const next: Record<string, Record<string, unknown>> = {};
    for (const mod of support?.mods ?? []) {
      next[mod.modId] = { ...mod.values };
    }
    drafts = next;
    dirtyMods = new Set();
  }

  function resetModDraft(modId: string) {
    const mod = support?.mods.find((candidate) => candidate.modId === modId);
    if (!mod) return;
    drafts = { ...drafts, [modId]: { ...mod.values } };
    dirtyMods = new Set([...dirtyMods].filter((candidate) => candidate !== modId));
  }

  function clearModError(modId: string) {
    modErrors = Object.fromEntries(
      Object.entries(modErrors).filter(([candidate]) => candidate !== modId)
    );
  }

  function localized(text?: LocalizedText): string {
    if (!text) return '';
    return $activeLanguageStore === 'de' ? (text.de ?? text.en) : text.en;
  }

  function runtimeLabel(game: SteamGame): string {
    const runtime = game.bepInEx.runtime;
    const architecture = game.bepInEx.architecture?.toUpperCase();
    if (!runtime || !architecture) return '';
    return `${$t(`steamGames.bepInEx.runtime.${runtime}`)} · ${architecture}`;
  }

  function reasonLabel(reason?: BepInExReason): string {
    return $t(`steamGames.bepInEx.reasons.${reason ?? 'inspectionFailed'}`);
  }

  async function showAction(action: PendingAction) {
    pendingAction = action;
    removeConfigOnUninstall = false;
    await tick();
    confirmationDialog.showModal();
  }

  async function prepareBepInstall() {
    if (!selectedGame) return;
    pageError = '';
    actionBusy = true;
    try {
      const plan = await prepareBepInExInstall(selectedGame.appId);
      await showAction({ kind: 'bepinexInstall', plan });
    } catch (error) {
      pageError = errorMessage(error);
    } finally {
      actionBusy = false;
    }
  }

  async function prepareBepUninstall() {
    if (!selectedGame) return;
    pageError = '';
    actionBusy = true;
    try {
      const plan = await prepareBepInExUninstall(selectedGame.appId);
      await showAction({ kind: 'bepinexUninstall', plan });
    } catch (error) {
      pageError = errorMessage(error);
    } finally {
      actionBusy = false;
    }
  }

  async function prepareModAction(mod: GameMod, action: 'install' | 'update' | 'uninstall') {
    if (!selectedGame) return;
    clearModError(mod.modId);
    busyModId = mod.modId;
    try {
      const plan =
        action === 'install'
          ? await prepareModInstall(selectedGame.appId, [mod.modId])
          : action === 'update'
            ? await prepareModUpdate(selectedGame.appId, mod.modId)
            : await prepareModUninstall(selectedGame.appId, mod.modId, false);
      await showAction({ kind: 'mod', plan, modId: mod.modId, modName: localized(mod.name) });
    } catch (error) {
      modErrors = { ...modErrors, [mod.modId]: errorMessage(error) };
    } finally {
      busyModId = undefined;
    }
  }

  async function prepareDevelopmentAgent() {
    if (!selectedGame) return;
    pageError = '';
    await showAction({ kind: 'developmentAgent', appId: selectedGame.appId });
  }

  function closeConfirmation() {
    confirmationDialog.close();
    pendingAction = undefined;
  }

  async function executeAction() {
    const action = pendingAction;
    if (!action) return;
    confirmationDialog.close();
    pendingAction = undefined;
    if (action.kind === 'mod') {
      busyModId = action.modId;
      clearModError(action.modId);
    } else {
      actionBusy = true;
      pageError = '';
    }
    try {
      if (action.kind === 'bepinexInstall') {
        await installBepInEx(action.plan.planId, (next) => (progress = next));
      } else if (action.kind === 'bepinexUninstall') {
        await uninstallBepInEx(action.plan.planId);
      } else if (action.kind === 'developmentAgent') {
        support = await installDevelopmentAgent(action.appId);
      } else if (action.plan.action === 'install') {
        support = await installMods(action.plan.planId);
      } else if (action.plan.action === 'update') {
        support = await updateMod(action.plan.planId);
      } else {
        const uninstallPlan =
          removeConfigOnUninstall && selectedGame
            ? await prepareModUninstall(
                selectedGame.appId,
                action.plan.modIds[0],
                true
              )
            : action.plan;
        support = await uninstallMod(uninstallPlan.planId);
      }
      await reloadDetail();
    } catch (error) {
      if (action.kind === 'mod') {
        modErrors = { ...modErrors, [action.modId]: errorMessage(error) };
      } else {
        pageError = errorMessage(error);
      }
    } finally {
      if (action.kind === 'mod') busyModId = undefined;
      else actionBusy = false;
      progress = undefined;
    }
  }

  function setDraft(modId: string, fieldId: string, value: unknown) {
    drafts = {
      ...drafts,
      [modId]: { ...(drafts[modId] ?? {}), [fieldId]: value }
    };
    dirtyMods = new Set([...dirtyMods, modId]);
  }

  function toggleMulti(modId: string, fieldId: string, option: string, checked: boolean) {
    const current = Array.isArray(drafts[modId]?.[fieldId])
      ? (drafts[modId][fieldId] as string[])
      : [];
    setDraft(
      modId,
      fieldId,
      checked ? [...new Set([...current, option])] : current.filter((value) => value !== option)
    );
  }

  async function saveConfig(mod: GameMod) {
    if (!selectedGame) return;
    busyModId = mod.modId;
    clearModError(mod.modId);
    try {
      support = await setModConfig(selectedGame.appId, mod.modId, drafts[mod.modId] ?? {});
      resetModDraft(mod.modId);
    } catch (error) {
      modErrors = { ...modErrors, [mod.modId]: errorMessage(error) };
    } finally {
      busyModId = undefined;
    }
  }

  function errorMessage(error: unknown): string {
    const code =
      typeof error === 'object' && error !== null && 'code' in error
        ? String((error as { code: unknown }).code)
        : '';
    return $t(`steamGames.errors.${code || 'generic'}`);
  }

  function actionTitle(action: PendingAction): string {
    if (action.kind === 'bepinexInstall') return $t('steamGames.bepInEx.confirmTitle');
    if (action.kind === 'bepinexUninstall') return $t('steamGames.bepInEx.uninstallTitle');
    if (action.kind === 'developmentAgent') return $t('steamGames.developerAgent.confirmTitle');
    return $t(`steamGames.mods.confirm.${action.plan.action}Title`);
  }

  function actionDescription(action: PendingAction): string {
    if (action.kind === 'bepinexInstall') {
      return $t('steamGames.bepInEx.confirmDescription', {
        game: selectedGame?.name ?? '',
        version: action.plan.version,
        runtime: action.plan.runtime,
        architecture: action.plan.architecture.toUpperCase()
      });
    }
    if (action.kind === 'developmentAgent') {
      return $t('steamGames.developerAgent.confirmDescription', { game: selectedGame?.name ?? '' });
    }
    if (action.kind === 'bepinexUninstall') {
      return $t('steamGames.bepInEx.uninstallDescription', {
        version: action.plan.version,
        count: action.plan.additionalFileCount
      });
    }
    return $t(`steamGames.mods.confirm.${action.plan.action}Description`, {
      mod: action.modName,
      count: action.plan.modIds.length
    });
  }
</script>

{#if selectedGame}
  {@const selectedRuntimeLabel = runtimeLabel(selectedGame)}
  <section class="single-panel glass-panel game-detail" aria-busy={supportLoading || actionBusy || busyModId !== undefined}>
    <button class="detail-back" type="button" on:click={closeGame}>
      <span class="material-symbols-rounded" aria-hidden="true">arrow_back</span>
      {$t('steamGames.detail.back')}
    </button>

    <div class="detail-heading">
      <div>
        <p class="eyebrow">{$t('steamGames.detail.eyebrow')}</p>
        <h1>{selectedGame.name}</h1>
        {#if selectedRuntimeLabel}<p>{selectedRuntimeLabel}</p>{/if}
      </div>
      <span class="agent-state-slot">
        {#if supportLoading}
          <span class="skeleton-line skeleton-agent-state" aria-hidden="true"></span>
        {:else if support}
          <span class:connected={support.agentStatus === 'connected'} class="agent-state">
            {$t(`steamGames.agent.${support.agentStatus}`)}
          </span>
        {/if}
      </span>
    </div>

    <article class="detail-card">
      <div class="detail-card-main">
        <div>
          <h2>BepInEx</h2>
          <p>
            {#if selectedGame.bepInEx.status === 'installed'}
              {$t('steamGames.bepInEx.installedVersion', {
                version: selectedGame.bepInEx.installedVersion ?? $t('steamGames.detail.unknownVersion')
              })}
            {:else if selectedGame.bepInEx.status === 'installable'}
              {$t('steamGames.detail.bepInExReady')}
            {:else}
              {reasonLabel(selectedGame.bepInEx.reason)}
            {/if}
          </p>
        </div>
        <div class="detail-actions">
          {#if selectedGame.bepInEx.status === 'installable'}
            <button class="primary-action" type="button" disabled={actionBusy || busyModId !== undefined} on:click={prepareBepInstall}>
              {$t('steamGames.bepInEx.install')}
            </button>
          {:else if selectedGame.bepInEx.status === 'installed' && selectedGame.bepInEx.managedByGameTweaks}
            <button class="secondary-action danger-action" type="button" disabled={actionBusy || busyModId !== undefined} on:click={prepareBepUninstall}>
              {$t('steamGames.bepInEx.uninstall')}
            </button>
          {:else if selectedGame.bepInEx.status === 'installed'}
            <span class="managed-note">{$t('steamGames.bepInEx.manualInstall')}</span>
          {/if}
          {#if $developerModeStore.enabled && selectedGame.bepInEx.status === 'installed'}
            <button class="secondary-action developer-agent-action" type="button" disabled={actionBusy || busyModId !== undefined} on:click={prepareDevelopmentAgent}>
              <span class="material-symbols-rounded" aria-hidden="true">developer_mode</span>
              {$t('steamGames.developerAgent.install')}
            </button>
          {/if}
        </div>
      </div>
      {#if progress}
        <InstallProgress {progress} />
      {/if}
    </article>

    {#if pageError}
      <div class="unsupported-state detail-state error" role="alert">
        <span class="material-symbols-rounded" aria-hidden="true">error</span>
        <h2>{$t('steamGames.detail.actionErrorTitle')}</h2>
        <p>{pageError}</p>
      </div>
    {/if}

    <div class="detail-content">
      {#if supportLoading}
        <div class="detail-skeleton" role="status" aria-live="polite">
          <span class="sr-only">{$t('steamGames.detail.loading')}</span>
          <div class="mods-heading skeleton-heading" aria-hidden="true">
            <div>
              <span class="skeleton-line skeleton-eyebrow"></span>
              <span class="skeleton-line skeleton-section-title"></span>
            </div>
          </div>
          <div class="mod-list" aria-hidden="true">
            <div class="mod-card mod-skeleton">
              <div class="mod-skeleton-copy">
                <span class="skeleton-line skeleton-mod-title"></span>
                <span class="skeleton-line skeleton-description"></span>
                <span class="skeleton-line skeleton-description short"></span>
              </div>
              <span class="skeleton-line skeleton-action"></span>
            </div>
            <div class="mod-card mod-skeleton">
              <div class="mod-skeleton-copy">
                <span class="skeleton-line skeleton-mod-title short"></span>
                <span class="skeleton-line skeleton-description"></span>
                <span class="skeleton-line skeleton-description medium"></span>
              </div>
              <span class="skeleton-line skeleton-action"></span>
            </div>
          </div>
        </div>
      {:else if support?.status === 'unsupported'}
        <div class="unsupported-state" role="status">
          <span class="material-symbols-rounded" aria-hidden="true">extension_off</span>
          <h2>{$t('steamGames.detail.unsupportedTitle')}</h2>
          <p>{$t('steamGames.detail.unsupportedDescription')}</p>
        </div>
      {:else if support?.status === 'unavailable'}
        <div class="unsupported-state error" role="alert">
          <h2>{$t('steamGames.detail.unavailableTitle')}</h2>
          <p>{$t('steamGames.detail.unavailableDescription')}</p>
          <button class="secondary-action" type="button" on:click={() => openGame(selectedGame!)}>
            {$t('steamGames.detail.retry')}
          </button>
        </div>
      {:else if support?.status === 'supported'}
        <div class="mods-heading">
          <div>
            <p class="eyebrow">{$t('steamGames.mods.eyebrow')}</p>
            <h2>{$t('steamGames.mods.title')}</h2>
          </div>
          {#if support.cached}<span class="catalog-cache">{$t('steamGames.mods.cached')}</span>{/if}
        </div>
        {#if support.mods.length === 0}
          <p class="game-list-status" role="status">{$t('steamGames.mods.empty')}</p>
        {:else}
          <div class="mod-list">
            {#each support.mods as mod (mod.modId)}
            <article class:busy={busyModId === mod.modId} class="mod-card">
              <header>
                <div>
                  <div class="mod-title-row">
                    <h3>{localized(mod.name)}</h3>
                    <span class:official={mod.official} class:external={mod.external} class="mod-badge">
                      {mod.official
                        ? $t('steamGames.mods.official')
                        : mod.external
                          ? $t('steamGames.mods.external')
                          : $t('steamGames.mods.community')}
                    </span>
                  </div>
                  <p>{localized(mod.description)}</p>
                  {#if mod.restartRequired}
                    <p class="restart-required">{$t('steamGames.mods.restartRequired')}</p>
                  {/if}
                </div>
                <span class="mod-version">
                  {#if busyModId === mod.modId}
                    <span class="material-symbols-rounded mod-spinner" aria-label={$t('steamGames.mods.working')}>progress_activity</span>
                  {/if}
                  v{mod.installedVersion ?? mod.version}
                </span>
              </header>

              {#if modErrors[mod.modId]}
                <p class="detail-alert error" role="alert">{modErrors[mod.modId]}</p>
              {/if}

              {#if modProgress[mod.modId] && modProgress[mod.modId].stage !== 'completed'}
                <InstallProgress progress={modProgress[mod.modId]} />
              {/if}

              {#if mod.dependencies.length || mod.conflicts.length}
                <div class="mod-relations">
                  {#if mod.dependencies.length}
                    <span>{$t('steamGames.mods.dependencies')}: {mod.dependencies.map((dependency) => dependency.modId).join(', ')}</span>
                  {/if}
                  {#if mod.conflicts.length}
                    <span>{$t('steamGames.mods.conflicts')}: {mod.conflicts.join(', ')}</span>
                  {/if}
                </div>
              {/if}

              <div class="mod-actions">
                {#if mod.status === 'notInstalled'}
                  <button class="primary-action" type="button" disabled={actionBusy || busyModId !== undefined || selectedGame.bepInEx.status !== 'installed'} on:click={() => prepareModAction(mod, 'install')}>
                    {$t('steamGames.mods.install')}
                  </button>
                {:else if mod.status === 'updateAvailable'}
                  <button class="primary-action" type="button" disabled={actionBusy || busyModId !== undefined} on:click={() => prepareModAction(mod, 'update')}>
                    {$t('steamGames.mods.update')}
                  </button>
                  <button class="secondary-action" type="button" disabled={actionBusy || busyModId !== undefined} on:click={() => prepareModAction(mod, 'uninstall')}>
                    {$t('steamGames.mods.uninstall')}
                  </button>
                {:else if mod.status === 'installed'}
                  <button class="secondary-action" type="button" disabled={actionBusy || busyModId !== undefined} on:click={() => prepareModAction(mod, 'uninstall')}>
                    {$t('steamGames.mods.uninstall')}
                  </button>
                {:else if mod.status === 'external'}
                  <span class="managed-note">{$t('steamGames.mods.externalNote')}</span>
                {:else}
                  <span class="managed-note">{$t('steamGames.mods.blocked')}</span>
                {/if}
              </div>

              {#if mod.config.length && mod.status !== 'notInstalled' && mod.status !== 'blocked'}
                <div class="mod-config">
                  {#each mod.config as field (field.id)}
                    <div class:locked={field.locked} class="config-field">
                      <span>
                        <strong>{localized(field.label)}</strong>
                        {#if field.description}<small>{localized(field.description)}</small>{/if}
                        {#if field.locked}<small>{$t('steamGames.mods.schemaConflict')}</small>{/if}
                      </span>
                      {#if field.control === 'boolean'}
                        <span class:switch-control={field.display === 'switch'} class="boolean-control">
                          <input aria-label={localized(field.label)} type="checkbox" disabled={field.locked || actionBusy || busyModId !== undefined} checked={Boolean(drafts[mod.modId]?.[field.id])} on:change={(event) => setDraft(mod.modId, field.id, event.currentTarget.checked)} />
                          {#if field.display === 'switch'}<span class="switch-track" aria-hidden="true"></span>{/if}
                        </span>
                      {:else if field.control === 'string'}
                        <input aria-label={localized(field.label)} type="text" disabled={field.locked || actionBusy || busyModId !== undefined} maxlength={field.maxLength} value={String(drafts[mod.modId]?.[field.id] ?? '')} on:input={(event) => setDraft(mod.modId, field.id, event.currentTarget.value)} />
                      {:else if field.control === 'integer' || field.control === 'decimal'}
                        <input aria-label={localized(field.label)} type="number" disabled={field.locked || actionBusy || busyModId !== undefined} min={field.min} max={field.max} step={field.step} value={Number(drafts[mod.modId]?.[field.id] ?? field.default)} on:input={(event) => setDraft(mod.modId, field.id, field.control === 'integer' ? Number.parseInt(event.currentTarget.value, 10) : Number.parseFloat(event.currentTarget.value))} />
                      {:else if field.control === 'singleSelect'}
                        {#if field.display === 'dropdown'}
                          <select aria-label={localized(field.label)} disabled={field.locked || actionBusy || busyModId !== undefined} value={String(drafts[mod.modId]?.[field.id] ?? field.default)} on:change={(event) => setDraft(mod.modId, field.id, event.currentTarget.value)}>
                            {#each field.options as option (option.value)}
                              <option value={option.value}>{localized(option.label)}</option>
                            {/each}
                          </select>
                        {:else}
                          <span class="multi-options">
                            {#each field.options as option (option.value)}
                              <label>
                                <input type="radio" name={`${mod.modId}-${field.id}`} value={option.value} disabled={field.locked || actionBusy || busyModId !== undefined} checked={String(drafts[mod.modId]?.[field.id] ?? field.default) === option.value} on:change={() => setDraft(mod.modId, field.id, option.value)} />
                                {localized(option.label)}
                              </label>
                            {/each}
                          </span>
                        {/if}
                      {:else if field.control === 'multiSelect'}
                        <span class="multi-options">
                          {#each field.options as option (option.value)}
                            <label>
                              <input type="checkbox" disabled={field.locked || actionBusy || busyModId !== undefined} checked={Array.isArray(drafts[mod.modId]?.[field.id]) && (drafts[mod.modId][field.id] as string[]).includes(option.value)} on:change={(event) => toggleMulti(mod.modId, field.id, option.value, event.currentTarget.checked)} />
                              {localized(option.label)}
                            </label>
                          {/each}
                        </span>
                      {/if}
                    </div>
                  {/each}
                  <div class="config-actions">
                    <button class="secondary-action" type="button" disabled={!dirtyMods.has(mod.modId) || actionBusy || busyModId !== undefined} on:click={() => resetModDraft(mod.modId)}>
                      {$t('steamGames.mods.reset')}
                    </button>
                    <button class="primary-action" type="button" disabled={!dirtyMods.has(mod.modId) || actionBusy || busyModId !== undefined} on:click={() => saveConfig(mod)}>
                      {$t('steamGames.mods.save')}
                    </button>
                  </div>
                </div>
              {/if}
              </article>
            {/each}
          </div>
        {/if}
      {/if}
    </div>
  </section>
{:else}
  <section class="single-panel glass-panel" aria-busy={loadState === 'loading'}>
    <SectionHeader eyebrow={$t('steamGames.eyebrow')} title={$t('steamGames.title')} description={$t('steamGames.description')} />
    {#if loadState === 'loading'}
      <div class="game-skeleton-list" role="status" aria-live="polite">
        <span class="sr-only">{$t('steamGames.loading')}</span>
        {#each [0, 1, 2] as row (row)}
          <div class="game-skeleton-row" aria-hidden="true">
            <span class="game-skeleton-copy">
              <span class:short={row === 1} class="skeleton-line skeleton-game-title"></span>
              <span class:medium={row === 2} class="skeleton-line skeleton-game-meta"></span>
            </span>
            <span class="skeleton-line skeleton-chevron"></span>
          </div>
        {/each}
      </div>
    {:else if loadState === 'error'}
      <p class="game-list-status error" role="alert">{$t('steamGames.error')}</p>
    {:else if games.length === 0}
      <p class="game-list-status" role="status">{$t('steamGames.empty')}</p>
    {:else}
      <ul class="game-list" aria-label={$t('steamGames.listLabel')}>
        {#each games as game (game.appId)}
          {@const gameRuntimeLabel = runtimeLabel(game)}
          <li>
            <button class="game-row" type="button" on:click={() => openGame(game)}>
              <span class="game-details">
                <strong>{game.name}</strong>
                {#if gameRuntimeLabel}<span class="game-runtime">{gameRuntimeLabel}</span>{/if}
                {#if game.bepInEx.status === 'unsupported' || game.bepInEx.status === 'blocked'}
                  <span class:blocked={game.bepInEx.status === 'blocked'} class="game-compatibility">{reasonLabel(game.bepInEx.reason)}</span>
                {/if}
              </span>
              <span class="game-row-end">
                {#if game.bepInEx.status === 'installed'}
                  <span class="game-installed-status"><span class="material-symbols-rounded" aria-hidden="true">check_circle</span>BepInEx</span>
                {/if}
                <span class="material-symbols-rounded" aria-hidden="true">chevron_right</span>
              </span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<dialog class="bepinex-confirmation glass-panel" bind:this={confirmationDialog} aria-labelledby="action-confirmation-title" on:cancel={(event) => { event.preventDefault(); closeConfirmation(); }}>
  {#if pendingAction}
    <div class="bepinex-confirmation-content">
      <div>
        <p class="eyebrow">GameTweaks</p>
        <h2 id="action-confirmation-title">{actionTitle(pendingAction)}</h2>
      </div>
      <p>{actionDescription(pendingAction)}</p>
      <p class="bepinex-confirmation-warning">
        <span class="material-symbols-rounded" aria-hidden="true">warning</span>
        <span>{$t('steamGames.bepInEx.closeGameWarning')}</span>
      </p>
      {#if pendingAction.kind === 'mod' && pendingAction.plan.action === 'uninstall'}
        <label class="remove-config-option">
          <input type="checkbox" bind:checked={removeConfigOnUninstall} />
          <span>{$t('steamGames.mods.removeConfig')}</span>
        </label>
      {/if}
      <div class="bepinex-confirmation-actions">
        <button class="secondary-action" type="button" on:click={closeConfirmation}>{$t('steamGames.bepInEx.cancel')}</button>
        <button class="primary-action" type="button" on:click={executeAction}>{$t('steamGames.detail.confirm')}</button>
      </div>
    </div>
  {/if}
</dialog>
