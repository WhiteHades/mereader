<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { errorMessage, getBook, updateProgress } from '../lib/commands';
  import { chapterContainsLocation, findChapterAtLocation } from '../lib/chapters';
  import { ProgressSaveQueue } from '../lib/progress';
  import type {
    BookDetail,
    ChapterSummary,
    ReaderSettings,
    ReaderTheme,
    SourcePassage,
  } from '../lib/types';
  import AiPanel from './AiPanel.svelte';
  import ReaderContent from './ReaderContent.svelte';

  interface Props {
    bookId: string;
    onBack: () => void;
  }

  interface ReaderContentApi {
    goToLocation: (location: number) => Promise<void>;
    goToChapter: (chapterId: string) => Promise<void>;
    previousChapter: () => Promise<void>;
    nextChapter: () => Promise<void>;
    previousViewport: () => Promise<void>;
    nextViewport: () => Promise<void>;
  }

  interface AiPanelApi {
    focusPanel: () => void;
  }

  const SETTINGS_KEY = 'mereader.reader-settings.v1';
  const defaultSettings: ReaderSettings = {
    theme: 'sepia',
    fontSize: 19,
    lineHeight: 1.7,
    readingWidth: 700,
  };

  let { bookId, onBack }: Props = $props();
  let detail = $state<BookDetail | null>(null);
  let loading = $state(true);
  let loadError = $state('');
  let saveError = $state('');
  let leaving = $state(false);
  let initialLocation = $state(1);
  let currentLocation = $state(1);
  let currentChapter = $state<ChapterSummary | null>(null);
  let tocOpen = $state(false);
  let aiOpen = $state(false);
  let settingsOpen = $state(false);
  let settings = $state<ReaderSettings>(loadSettings());
  let readerContent = $state<ReaderContentApi>();
  let aiPanel = $state<AiPanelApi>();

  let completion = $derived(
    detail?.totalLocations
      ? Math.max(0, Math.min(100, (currentLocation / detail.totalLocations) * 100))
      : 0,
  );

  const saveQueue = new ProgressSaveQueue(
    async (position) => {
      await updateProgress(
        position.bookId,
        position.currentLocation,
        position.currentChapterId,
      );
      saveError = '';
    },
    (error) => {
      saveError = `Reading position is not saved yet. ${errorMessage(error)}`;
    },
  );

  $effect(() => {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
  });

  onMount(() => {
    void loadBook();
  });

  async function loadBook(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      const loaded = await getBook(bookId);
      loaded.chapters = [...loaded.chapters].sort((left, right) => left.order - right.order);
      detail = loaded;
      initialLocation = loaded.progress?.currentLocation ?? loaded.chapters[0]?.startLocation ?? 1;
      currentLocation = initialLocation;
      currentChapter =
        loaded.chapters.find(
          (chapter) => chapter.id === loaded.progress?.currentChapterId &&
            chapterContainsLocation(loaded.chapters, chapter, initialLocation),
        ) ??
        findChapterAtLocation(loaded.chapters, initialLocation) ??
        loaded.chapters[0] ??
        null;
    } catch (error) {
      loadError = errorMessage(error);
    } finally {
      loading = false;
    }
  }

  function positionChanged(position: { location: number; chapterId: string }): void {
    currentLocation = position.location;
    saveQueue.enqueue({
      bookId,
      currentLocation: position.location,
      currentChapterId: position.chapterId,
    });
  }

  async function handleBack(): Promise<void> {
    leaving = true;
    try {
      await saveQueue.flush();
      onBack();
    } catch {
      // The queue keeps the latest position pending so the user can retry without data loss.
    } finally {
      leaving = false;
    }
  }

  async function retrySave(): Promise<void> {
    try {
      await saveQueue.flush();
    } catch {
      // The queue reports the concrete core error through saveError.
    }
  }

  async function openToc(): Promise<void> {
    tocOpen = !tocOpen;
    if (tocOpen && narrowViewport()) aiOpen = false;
  }

  async function openAi(): Promise<void> {
    aiOpen = !aiOpen;
    if (aiOpen && narrowViewport()) tocOpen = false;
    if (aiOpen) {
      await tick();
      aiPanel?.focusPanel();
    }
  }

  async function selectChapter(chapter: ChapterSummary): Promise<void> {
    await readerContent?.goToChapter(chapter.id);
    currentChapter = chapter;
    tocOpen = false;
  }

  async function jumpToSource(source: SourcePassage): Promise<void> {
    if (source.startLocation === undefined) return;
    await readerContent?.goToLocation(source.startLocation);
    aiOpen = false;
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key !== 'Escape') return;
    if (aiOpen || tocOpen || settingsOpen) {
      event.preventDefault();
      aiOpen = false;
      tocOpen = false;
      settingsOpen = false;
    }
  }

  function updateSetting<Key extends keyof ReaderSettings>(
    key: Key,
    value: ReaderSettings[Key],
  ): void {
    settings = { ...settings, [key]: value };
  }

  function loadSettings(): ReaderSettings {
    try {
      const stored = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? '{}') as Partial<ReaderSettings>;
      const theme: ReaderTheme = ['light', 'sepia', 'dark'].includes(stored.theme ?? '')
        ? (stored.theme as ReaderTheme)
        : defaultSettings.theme;
      return {
        theme,
        fontSize: clampNumber(stored.fontSize, 14, 28, defaultSettings.fontSize),
        lineHeight: clampNumber(stored.lineHeight, 1.3, 2.1, defaultSettings.lineHeight),
        readingWidth: clampNumber(stored.readingWidth, 520, 920, defaultSettings.readingWidth),
      };
    } catch {
      return { ...defaultSettings };
    }
  }

  function clampNumber(value: unknown, minimum: number, maximum: number, fallback: number): number {
    return typeof value === 'number' && Number.isFinite(value)
      ? Math.max(minimum, Math.min(maximum, value))
      : fallback;
  }

  function narrowViewport(): boolean {
    return window.matchMedia('(max-width: 780px)').matches;
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<section class={`reader theme-${settings.theme}`} aria-label="Reader">
  {#if loading}
    <div class="reader-load-state" role="status" aria-live="polite">
      <span class="activity-dot" aria-hidden="true"></span>
      <p>Restoring your reading position</p>
    </div>
  {:else if loadError}
    <div class="state-panel error-state" role="alert">
      <p class="state-code">CORE / BOOK</p>
      <h1>Book unavailable</h1>
      <p>{loadError}</p>
      <div class="state-actions">
        <button class="button primary" onclick={loadBook}>Retry</button>
        <button class="button quiet" onclick={onBack}>Back to library</button>
      </div>
    </div>
  {:else if detail}
    <header class="reader-topbar">
      <button class="button quiet back-to-library" onclick={handleBack} disabled={leaving}>
        {leaving ? 'Saving...' : 'Library'}
      </button>
      <div class="reader-title">
        <strong>{detail.title}</strong>
        <span>{currentChapter?.title ?? 'Reading'}</span>
      </div>
      <div class="reader-actions">
        <button class="button quiet" onclick={openToc} aria-expanded={tocOpen} aria-controls="toc-drawer">Contents</button>
        <button class="button quiet" onclick={openAi} aria-expanded={aiOpen} aria-controls="ai-drawer">Ask AI</button>
        <button class="button quiet" onclick={() => (settingsOpen = !settingsOpen)} aria-expanded={settingsOpen} aria-controls="reader-settings">Text</button>
      </div>
    </header>

    {#if saveError}
      <div class="save-warning" role="alert">
        <span>{saveError}</span>
        <button class="text-button" onclick={retrySave}>Retry save</button>
      </div>
    {/if}

    <ReaderContent
      bind:this={readerContent}
      bookId={detail.id}
      chapters={detail.chapters}
      {initialLocation}
      {settings}
      onPositionChange={positionChanged}
      onChapterChange={(chapter) => (currentChapter = chapter)}
    />

    <aside id="toc-drawer" class:drawer-open={tocOpen} class="drawer toc-drawer" hidden={!tocOpen} aria-hidden={!tocOpen}>
      <header class="drawer-header">
        <div>
          <p class="eyebrow">{detail.chapters.length} chapters</p>
          <h2>Contents</h2>
        </div>
        <button class="icon-button" onclick={() => (tocOpen = false)} aria-label="Close contents panel">Close</button>
      </header>
      <nav aria-label="Table of contents">
        <ol class="toc-list">
          {#each detail.chapters as chapter}
            <li>
              <button class:active={chapter.id === currentChapter?.id} onclick={() => selectChapter(chapter)}>
                <span>{String(chapter.order).padStart(2, '0')}</span>
                <strong>{chapter.title}</strong>
              </button>
            </li>
          {/each}
        </ol>
      </nav>
    </aside>

    <div id="ai-drawer">
      <AiPanel
        bind:this={aiPanel}
        bookId={detail.id}
        open={aiOpen}
        onClose={() => (aiOpen = false)}
        onJumpToSource={jumpToSource}
      />
    </div>

    {#if settingsOpen}
      <section class="settings-popover" id="reader-settings" aria-label="Reader settings">
        <header>
          <strong>Reading settings</strong>
          <button class="icon-button" onclick={() => (settingsOpen = false)} aria-label="Close reading settings">Close</button>
        </header>
        <label>
          <span>Theme</span>
          <select value={settings.theme} onchange={(event) => updateSetting('theme', event.currentTarget.value as ReaderTheme)}>
            <option value="light">Paper</option>
            <option value="sepia">Warm</option>
            <option value="dark">Night</option>
          </select>
        </label>
        <label>
          <span>Font size <output>{settings.fontSize}px</output></span>
          <input type="range" min="14" max="28" step="1" value={settings.fontSize} oninput={(event) => updateSetting('fontSize', Number(event.currentTarget.value))} />
        </label>
        <label>
          <span>Line height <output>{settings.lineHeight.toFixed(1)}</output></span>
          <input type="range" min="1.3" max="2.1" step="0.1" value={settings.lineHeight} oninput={(event) => updateSetting('lineHeight', Number(event.currentTarget.value))} />
        </label>
        <label>
          <span>Reading width <output>{settings.readingWidth}px</output></span>
          <input type="range" min="520" max="920" step="20" value={settings.readingWidth} oninput={(event) => updateSetting('readingWidth', Number(event.currentTarget.value))} />
        </label>
        <p class="position-note">Position uses the core's stable chapter location range. Exact DOM anchors or EPUB CFI require a future resolver contract.</p>
      </section>
    {/if}

    <footer class="reader-controls" aria-label="Reading navigation">
      <button class="control-button" onclick={() => readerContent?.previousChapter()} disabled={detail.chapters[0]?.id === currentChapter?.id}>Previous chapter</button>
      <button class="control-button compact" onclick={() => readerContent?.previousViewport()} aria-label="Previous viewport">Page up</button>
      <div class="reader-progress" aria-label={`${Math.round(completion)}% read`}>
        <span>{Math.round(completion)}%</span>
        <progress max="100" value={completion}>{Math.round(completion)}%</progress>
      </div>
      <button class="control-button compact" onclick={() => readerContent?.nextViewport()} aria-label="Next viewport">Page down</button>
      <button class="control-button" onclick={() => readerContent?.nextChapter()} disabled={detail.chapters.at(-1)?.id === currentChapter?.id}>Next chapter</button>
    </footer>
  {/if}
</section>
