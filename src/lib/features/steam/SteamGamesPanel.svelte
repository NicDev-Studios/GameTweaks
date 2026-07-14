<script lang="ts">
  import { onMount } from 'svelte';
  import SectionHeader from '$lib/components/SectionHeader.svelte';
  import { listSteamGames, type SteamGame } from '$lib/api/steam';
  import { t } from '$lib/i18n';

  type LoadState = 'loading' | 'ready' | 'error';

  let games: SteamGame[] = [];
  let loadState: LoadState = 'loading';

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
        <li class="game-row">{game.name}</li>
      {/each}
    </ul>
  {/if}
</section>
