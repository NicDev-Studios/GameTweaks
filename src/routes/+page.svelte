<script lang="ts">
  import { onMount } from 'svelte';
  import AppShell from '$lib/components/AppShell.svelte';
  import SettingsPanel from '$lib/features/settings/SettingsPanel.svelte';
  import { updaterStore } from '$lib/stores/updater';

  onMount(() => {
    void updaterStore.initialize();
    void updaterStore.checkForUpdatesOnStartup();
  });
</script>

<AppShell>
  <SettingsPanel />
</AppShell>
