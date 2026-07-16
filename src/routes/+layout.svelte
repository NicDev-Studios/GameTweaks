<script lang="ts">
  import '../lib/styles/global.css';
  import { onMount } from 'svelte';
  import StartupIntro from '$lib/components/startup/StartupIntro.svelte';
  import { t } from '$lib/i18n';
  import { languageStore } from '$lib/stores/language';
  import { developerModeStore } from '$lib/stores/developerMode';
  import { themeStore } from '$lib/stores/theme';

  onMount(() => {
    function syncFocusState() {
      globalThis.document.documentElement.classList.toggle(
        'app-unfocused',
        !globalThis.document.hasFocus()
      );
    }

    languageStore.init();
    developerModeStore.init();
    themeStore.init();
    syncFocusState();

    globalThis.addEventListener('focus', syncFocusState);
    globalThis.addEventListener('blur', syncFocusState);

    return () => {
      globalThis.removeEventListener('focus', syncFocusState);
      globalThis.removeEventListener('blur', syncFocusState);
    };
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
