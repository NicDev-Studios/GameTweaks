<script lang="ts">
  import '../lib/styles/global.css';
  import { onMount } from 'svelte';
  import StartupIntro from '$lib/components/startup/StartupIntro.svelte';
  import { t } from '$lib/i18n';
  import { languageStore } from '$lib/stores/language';
  import { developerModeStore } from '$lib/stores/developerMode';
  import { themeStore } from '$lib/stores/theme';

  onMount(() => {
    languageStore.init();
    developerModeStore.init();
    themeStore.init();
  });
</script>

<svelte:head>
  <title>GameTweaks</title>
  <meta
    name="description"
    content={$t('app.description')}
  />
</svelte:head>

<StartupIntro />
<slot />
