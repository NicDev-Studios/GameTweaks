<script lang="ts">
  import { onMount } from 'svelte';

  export let visible = true;

  const totalDurationMs = 1650;
  const reducedDurationMs = 260;

  onMount(() => {
    const reduceMotion = globalThis.matchMedia('(prefers-reduced-motion: reduce)').matches;
    const duration = reduceMotion ? reducedDurationMs : totalDurationMs;
    const timer = globalThis.setTimeout(() => {
      visible = false;
    }, duration);

    return () => globalThis.clearTimeout(timer);
  });
</script>

{#if visible}
  <div class="startup-intro" aria-label="GameTweaks startet">
    <div class="startup-glass">
      <div class="startup-logo" aria-hidden="true">
        <span>G</span>
      </div>
      <div class="startup-copy">
        <strong>GameTweaks</strong>
        <span>Game customization, made simple</span>
      </div>
      <div class="startup-progress" aria-hidden="true">
        <span></span>
      </div>
    </div>
  </div>
{/if}
