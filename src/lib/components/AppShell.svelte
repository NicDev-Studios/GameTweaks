<script lang="ts">
  import { t } from '$lib/i18n';
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
      <button class="active" type="button" aria-current="page">
        <span class="material-symbols-rounded" aria-hidden="true">tune</span>
        {$t('navigation.settings')}
      </button>
    </nav>
  </aside>

  <main class="workspace">
    <slot />
  </main>
</div>
