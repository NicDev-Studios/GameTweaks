<script lang="ts">
  import { onMount } from 'svelte';
  import AppShell from '$lib/components/AppShell.svelte';
  import SteamGamesPanel from '$lib/features/steam/SteamGamesPanel.svelte';
  import SettingsPanel from '$lib/features/settings/SettingsPanel.svelte';
  import { updaterStore } from '$lib/stores/updater';

  type AppView = 'games' | 'settings';

  let activeView: AppView = 'games';

  onMount(() => {
    void updaterStore.initialize();
    void updaterStore.checkForUpdatesOnStartup();
  });

  function navigate(view: AppView) {
    activeView = view;
  }
</script>

<AppShell {activeView} onNavigate={navigate}>
  {#if activeView === 'games'}
    <SteamGamesPanel />
  {:else}
    <SettingsPanel />
  {/if}
</AppShell>
