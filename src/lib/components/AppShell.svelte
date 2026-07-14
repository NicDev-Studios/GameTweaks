<script lang="ts">
  import { t } from '$lib/i18n';

  export let activeView: 'games' | 'settings';
  export let onNavigate: (view: 'games' | 'settings') => void;
</script>

<div class="app-shell">
  <aside class="sidebar glass-panel">
    <div class="brand">
      <div class="brand-mark">G</div>
      <div>
        <strong>GameTweaks</strong>
        <span>{$t('navigation.tagline')}</span>
      </div>
    </div>

    <nav aria-label={$t('navigation.primary')}>
      <button
        class:active={activeView === 'games'}
        type="button"
        aria-current={activeView === 'games' ? 'page' : undefined}
        on:click={() => onNavigate('games')}
      >
        <span class="material-symbols-rounded" aria-hidden="true">sports_esports</span>
        {$t('navigation.games')}
      </button>
      <button
        class:active={activeView === 'settings'}
        type="button"
        aria-current={activeView === 'settings' ? 'page' : undefined}
        on:click={() => onNavigate('settings')}
      >
        <span class="material-symbols-rounded" aria-hidden="true">tune</span>
        {$t('navigation.settings')}
      </button>
    </nav>
  </aside>

  <main class="workspace">
    <slot />
  </main>
</div>
