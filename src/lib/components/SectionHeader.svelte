<script lang="ts">
  export let title: string;
  export let eyebrow: string;
  export let description: string;
</script>

<header class="section-header">
  <span>{eyebrow}</span>
  <div>
    <h1>{title}</h1>
    <p>{description}</p>
  </div>
</header>

