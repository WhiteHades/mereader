<script lang="ts">
  import Library from './components/Library.svelte';
  import Reader from './components/Reader.svelte';

  let selectedBookId = $state<string | null>(null);
</script>

<main class="app-shell">
  {#if selectedBookId}
    <Reader bookId={selectedBookId} onBack={() => (selectedBookId = null)} />
  {:else}
    <Library onOpenBook={(bookId) => (selectedBookId = bookId)} />
  {/if}
</main>
