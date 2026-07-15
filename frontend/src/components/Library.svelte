<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import iconUrl from '../assets/mereader_icon.png';
  import {
    deleteBook,
    errorMessage,
    getCover,
    importBook,
    listBooks,
  } from '../lib/commands';
  import type { BookSummary } from '../lib/types';

  interface Props {
    onOpenBook: (bookId: string) => void;
  }

  let { onOpenBook }: Props = $props();

  let books = $state<BookSummary[]>([]);
  let loading = $state(true);
  let loadError = $state('');
  let importState = $state<'idle' | 'importing'>('idle');
  let importError = $state('');
  let actionError = $state('');
  let query = $state('');
  let sort = $state<'title' | 'author' | 'progress'>('title');
  let deleteCandidate = $state<string | null>(null);
  let deletingBookId = $state<string | null>(null);
  let coverUrls = $state<Record<string, string>>({});
  const requestedCovers = new Set<string>();

  let visibleBooks = $derived.by(() => {
    const needle = query.trim().toLocaleLowerCase();
    const filtered = needle
      ? books.filter((book) =>
          `${book.title} ${book.author ?? ''}`.toLocaleLowerCase().includes(needle),
        )
      : books;

    return [...filtered].sort((left, right) => {
      if (sort === 'progress') {
        return progressPercent(right) - progressPercent(left) || left.title.localeCompare(right.title);
      }
      const leftValue = sort === 'author' ? left.author ?? '' : left.title;
      const rightValue = sort === 'author' ? right.author ?? '' : right.title;
      return leftValue.localeCompare(rightValue);
    });
  });

  onMount(() => {
    void loadLibrary();
  });

  onDestroy(() => {
    for (const url of Object.values(coverUrls)) URL.revokeObjectURL(url);
  });

  async function loadLibrary(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      const response = await listBooks();
      books = response.books;
    } catch (error) {
      loadError = errorMessage(error);
    } finally {
      loading = false;
    }
  }

  async function chooseBook(): Promise<void> {
    importError = '';
    importState = 'importing';
    try {
      const imported = await importBook();
      if (!imported) return;
      books = [imported, ...books.filter((book) => book.id !== imported.id)];
    } catch (error) {
      importError = errorMessage(error);
    } finally {
      importState = 'idle';
    }
  }

  async function confirmDelete(bookId: string): Promise<void> {
    deletingBookId = bookId;
    actionError = '';
    try {
      const deleted = await deleteBook(bookId);
      if (!deleted) {
        deleteCandidate = null;
        return;
      }
      const url = coverUrls[bookId];
      if (url) URL.revokeObjectURL(url);
      const nextUrls = { ...coverUrls };
      delete nextUrls[bookId];
      coverUrls = nextUrls;
      books = books.filter((book) => book.id !== bookId);
      deleteCandidate = null;
    } catch (error) {
      actionError = `Could not delete the book. ${errorMessage(error)}`;
    } finally {
      deletingBookId = null;
    }
  }

  function lazyCover(node: HTMLElement, bookId: string): { destroy: () => void } {
    const observer = new IntersectionObserver((entries) => {
      if (!entries.some((entry) => entry.isIntersecting)) return;
      observer.disconnect();
      void loadCover(bookId);
    }, { rootMargin: '160px' });
    observer.observe(node);
    return { destroy: () => observer.disconnect() };
  }

  async function loadCover(bookId: string): Promise<void> {
    if (requestedCovers.has(bookId) || coverUrls[bookId]) return;
    requestedCovers.add(bookId);
    try {
      const cover = await getCover(bookId);
      const url = URL.createObjectURL(new Blob([new Uint8Array(cover.data)], { type: cover.mimeType }));
      coverUrls = { ...coverUrls, [bookId]: url };
    } catch {
      // A missing cover is represented by the deterministic title placeholder.
    }
  }

  function progressPercent(book: BookSummary): number {
    return Math.max(0, Math.min(100, book.progress?.completionPercentage ?? 0));
  }

  function initials(title: string): string {
    const words = title.trim().split(/\s+/).filter(Boolean);
    return (words.length > 1 ? `${words[0][0]}${words[1][0]}` : words[0]?.slice(0, 2) || '?').toUpperCase();
  }

  function coverHue(title: string): number {
    return [...title].reduce((value, character) => value + character.charCodeAt(0), 0) % 360;
  }
</script>

<section class="library" aria-labelledby="library-title">
  <header class="library-header">
    <div class="brand-lockup">
      <img src={iconUrl} alt="" width="48" height="48" />
      <div>
        <p class="eyebrow">Personal reading room</p>
        <h1 id="library-title">MeReader</h1>
      </div>
    </div>
    <button class="button primary" onclick={chooseBook} disabled={importState !== 'idle'}>
      {importState === 'importing' ? 'Opening book...' : 'Import EPUB'}
    </button>
  </header>

  {#if importState !== 'idle'}
    <div class="status-line" role="status">
      <span class="activity-dot" aria-hidden="true"></span>
      The reader core is opening or importing your book
    </div>
  {/if}

  {#if importError}
    <div class="notice error-notice" role="alert">
      <div><strong>Import failed.</strong> {importError}</div>
      <button class="button quiet" onclick={chooseBook}>Try import again</button>
    </div>
  {/if}

  {#if actionError}
    <div class="notice error-notice" role="alert">
      <span>{actionError}</span>
      <button class="button quiet" onclick={() => (actionError = '')}>Dismiss</button>
    </div>
  {/if}

  {#if loading}
    <div class="library-loading" aria-live="polite" aria-busy="true">
      <p class="eyebrow">Opening library</p>
      <div class="skeleton-grid" aria-hidden="true">
        {#each Array(5) as _}
          <div class="book-skeleton"><span></span><i></i><i></i></div>
        {/each}
      </div>
    </div>
  {:else if loadError}
    <div class="state-panel error-state" role="alert">
      <p class="state-code">CORE / LIBRARY</p>
      <h2>Your library could not be opened</h2>
      <p>{loadError}</p>
      <button class="button primary" onclick={loadLibrary}>Retry</button>
    </div>
  {:else if books.length === 0}
    <div class="state-panel empty-state">
      <div class="empty-mark" aria-hidden="true">M</div>
      <p class="eyebrow">No books yet</p>
      <h2>Bring one book. Start somewhere.</h2>
      <p>Choose an EPUB from your computer. The native reader core opens and imports it locally.</p>
      <button class="button primary" onclick={chooseBook}>Import your first EPUB</button>
    </div>
  {:else}
    <div class="library-toolbar">
      <label class="search-field">
        <span>Search library</span>
        <input type="search" bind:value={query} placeholder="Title or author" />
      </label>
      <label class="sort-field">
        <span>Sort</span>
        <select bind:value={sort}>
          <option value="title">Title</option>
          <option value="author">Author</option>
          <option value="progress">Progress</option>
        </select>
      </label>
      <p class="book-count">{visibleBooks.length} of {books.length}</p>
    </div>

    {#if visibleBooks.length === 0}
      <div class="search-empty" role="status">
        <h2>No matching books</h2>
        <p>Try a shorter title or clear the search.</p>
        <button class="button quiet" onclick={() => (query = '')}>Clear search</button>
      </div>
    {:else}
      <div class="book-grid">
        {#each visibleBooks as book (book.id)}
          <article class="book-card">
            <button class="book-open" onclick={() => onOpenBook(book.id)} aria-label={`Read ${book.title}`}>
              <span
                class="book-cover"
                use:lazyCover={book.id}
                style={`--cover-hue: ${coverHue(book.title)}`}
              >
                {#if coverUrls[book.id]}
                  <img src={coverUrls[book.id]} alt={`Cover of ${book.title}`} loading="lazy" />
                {:else}
                  <span class="cover-initials" aria-hidden="true">{initials(book.title)}</span>
                {/if}
                {#if progressPercent(book) > 0}
                  <span class="cover-progress">{Math.round(progressPercent(book))}%</span>
                {/if}
              </span>
              <span class="book-meta">
                <strong>{book.title}</strong>
                <span>{book.author || 'Unknown author'}</span>
              </span>
            </button>
            <div class="book-progress" aria-label={`${Math.round(progressPercent(book))}% read`}>
              <span style={`width: ${progressPercent(book)}%`}></span>
            </div>
            {#if deleteCandidate === book.id}
              <div class="delete-confirm" role="group" aria-label={`Delete ${book.title}?`}>
                <span>Remove this book?</span>
                <button class="text-button danger" onclick={() => confirmDelete(book.id)} disabled={deletingBookId === book.id}>
                  {deletingBookId === book.id ? 'Removing...' : 'Remove'}
                </button>
                <button class="text-button" onclick={() => (deleteCandidate = null)}>Keep</button>
              </div>
            {:else}
              <button class="text-button remove-book" onclick={() => (deleteCandidate = book.id)} aria-label={`Delete ${book.title}`}>
                Remove
              </button>
            {/if}
          </article>
        {/each}
      </div>
    {/if}
  {/if}
</section>
