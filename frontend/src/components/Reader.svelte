<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import { errorMessage, getBook, updateProgress } from '../lib/commands';
  import { chapterContainsLocation, findChapterAtLocation } from '../lib/chapters';
  import { ProgressSaveQueue } from '../lib/progress';
  import type {
    BookDetail,
    ChapterSummary,
    ReaderSettings,
    ReaderTheme,
    ProgressPosition,
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
    focusContent: () => void;
  }

  interface AiPanelApi {
    focusPanel: () => void;
  }

  const SETTINGS_KEY = 'mereader.reader-settings.v1';
  const SAVE_DEBOUNCE_MS = 160;
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
  let initialLocation = $state(0);
  let currentLocation = $state(0);
  let currentChapter = $state<ChapterSummary | null>(null);
  let tocOpen = $state(false);
  let aiOpen = $state(false);
  let settingsOpen = $state(false);
  let settings = $state<ReaderSettings>(loadSettings());
  let readerContent = $state<ReaderContentApi>();
  let aiPanel = $state<AiPanelApi>();
  let tocTrigger = $state<HTMLButtonElement>();
  let aiTrigger = $state<HTMLButtonElement>();
  let settingsTrigger = $state<HTMLButtonElement>();
  let tocPanel = $state<HTMLElement>();
  let settingsPanel = $state<HTMLElement>();
  let isNarrow = $state(false);
  let lastOpenedPanel: 'toc' | 'ai' | 'settings' | null = null;
  let pendingPosition: ProgressPosition | null = null;
  let lastQueuedPosition: ProgressPosition | null = null;
  let saveTimer = 0;

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

  onMount(() => {
    const media = window.matchMedia('(max-width: 780px)');
    const updateViewport = (): void => {
      isNarrow = media.matches;
    };
    updateViewport();
    media.addEventListener('change', updateViewport);
    return () => media.removeEventListener('change', updateViewport);
  });

  onDestroy(() => {
    window.clearTimeout(saveTimer);
  });

  async function loadBook(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      const loaded = await getBook(bookId);
      loaded.chapters = [...loaded.chapters].sort((left, right) => left.order - right.order);
      detail = loaded;
      initialLocation = loaded.progress?.currentLocation ?? loaded.chapters[0]?.startLocation ?? 0;
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
    const nextPosition = {
      bookId,
      currentLocation: position.location,
      currentChapterId: position.chapterId,
    };
    if (samePosition(pendingPosition, nextPosition)) return;
    if (samePosition(lastQueuedPosition, nextPosition)) {
      pendingPosition = null;
      window.clearTimeout(saveTimer);
      saveTimer = 0;
      return;
    }
    pendingPosition = nextPosition;
    if (leaving) {
      commitPendingPosition();
      return;
    }
    window.clearTimeout(saveTimer);
    saveTimer = window.setTimeout(commitPendingPosition, SAVE_DEBOUNCE_MS);
  }

  function commitPendingPosition(): void {
    window.clearTimeout(saveTimer);
    saveTimer = 0;
    if (!pendingPosition) return;
    const position = pendingPosition;
    pendingPosition = null;
    lastQueuedPosition = position;
    saveQueue.enqueue(position);
  }

  function samePosition(
    left: ProgressPosition | null,
    right: ProgressPosition,
  ): boolean {
    return left?.bookId === right.bookId &&
      left.currentLocation === right.currentLocation &&
      left.currentChapterId === right.currentChapterId;
  }

  async function handleBack(): Promise<void> {
    leaving = true;
    try {
      commitPendingPosition();
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
      commitPendingPosition();
      await saveQueue.flush();
    } catch {
      // The queue reports the concrete core error through saveError.
    }
  }

  async function openToc(): Promise<void> {
    if (tocOpen) {
      await closeToc();
      return;
    }
    tocOpen = true;
    lastOpenedPanel = 'toc';
    if (isNarrow) {
      aiOpen = false;
      settingsOpen = false;
    }
    await tick();
    tocPanel?.focus();
  }

  async function openAi(): Promise<void> {
    if (aiOpen) {
      await closeAi();
      return;
    }
    aiOpen = true;
    lastOpenedPanel = 'ai';
    if (isNarrow) {
      tocOpen = false;
      settingsOpen = false;
    }
    await tick();
    aiPanel?.focusPanel();
  }

  async function openSettings(): Promise<void> {
    if (settingsOpen) {
      await closeSettings();
      return;
    }
    settingsOpen = true;
    lastOpenedPanel = 'settings';
    if (isNarrow) {
      tocOpen = false;
      aiOpen = false;
    }
    await tick();
    settingsPanel?.focus();
  }

  async function closeToc(restoreFocus = true): Promise<void> {
    tocOpen = false;
    if (lastOpenedPanel === 'toc') lastOpenedPanel = null;
    await tick();
    if (restoreFocus) tocTrigger?.focus();
  }

  async function closeAi(restoreFocus = true): Promise<void> {
    aiOpen = false;
    if (lastOpenedPanel === 'ai') lastOpenedPanel = null;
    await tick();
    if (restoreFocus) aiTrigger?.focus();
  }

  async function closeSettings(restoreFocus = true): Promise<void> {
    settingsOpen = false;
    if (lastOpenedPanel === 'settings') lastOpenedPanel = null;
    await tick();
    if (restoreFocus) settingsTrigger?.focus();
  }

  async function selectChapter(chapter: ChapterSummary): Promise<void> {
    await readerContent?.goToChapter(chapter.id);
    currentChapter = chapter;
    await closeToc(false);
    readerContent?.focusContent();
  }

  async function jumpToSource(source: SourcePassage): Promise<void> {
    if (source.startLocation === undefined) return;
    await readerContent?.goToLocation(source.startLocation);
    await closeAi(false);
    readerContent?.focusContent();
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key !== 'Escape') return;
    const panel = lastOpenedPanel && panelIsOpen(lastOpenedPanel)
      ? lastOpenedPanel
      : settingsOpen
        ? 'settings'
        : aiOpen
          ? 'ai'
          : tocOpen
            ? 'toc'
            : null;
    if (!panel) return;
    event.preventDefault();
    if (panel === 'settings') void closeSettings();
    if (panel === 'ai') void closeAi();
    if (panel === 'toc') void closeToc();
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

  function panelIsOpen(panel: 'toc' | 'ai' | 'settings'): boolean {
    return panel === 'toc' ? tocOpen : panel === 'ai' ? aiOpen : settingsOpen;
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
        <button bind:this={tocTrigger} class="button quiet" onclick={openToc} aria-expanded={tocOpen} aria-controls="toc-drawer">Contents</button>
        <button bind:this={aiTrigger} class="button quiet" onclick={openAi} aria-expanded={aiOpen} aria-controls="ai-panel">Ask AI</button>
        <button bind:this={settingsTrigger} class="button quiet" onclick={openSettings} aria-expanded={settingsOpen} aria-controls="reader-settings">Text</button>
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
      initialChapterId={currentChapter?.id ?? null}
      {settings}
      onPositionChange={positionChanged}
      onChapterChange={(chapter) => (currentChapter = chapter)}
    />

    <div
      id="toc-drawer"
      class:drawer-open={tocOpen}
      class="drawer toc-drawer"
      role="dialog"
      aria-labelledby="toc-panel-title"
      aria-modal={isNarrow ? 'true' : undefined}
      hidden={!tocOpen}
      aria-hidden={!tocOpen}
      inert={!tocOpen}
      tabindex="-1"
      bind:this={tocPanel}
    >
      <header class="drawer-header">
        <div>
          <p class="eyebrow">{detail.chapters.length} chapters</p>
          <h2 id="toc-panel-title">Contents</h2>
        </div>
        <button class="icon-button" onclick={() => void closeToc()} aria-label="Close contents panel">Close</button>
      </header>
      <nav aria-label="Table of contents">
        <ol class="toc-list">
          {#each detail.chapters as chapter, index}
            <li>
              <button class:active={chapter.id === currentChapter?.id} onclick={() => selectChapter(chapter)}>
                <span>{String(index + 1).padStart(2, '0')}</span>
                <strong>{chapter.title}</strong>
              </button>
            </li>
          {/each}
        </ol>
      </nav>
    </div>

    <div>
      <AiPanel
        bind:this={aiPanel}
        bookId={detail.id}
        open={aiOpen}
        modal={isNarrow}
        onClose={() => void closeAi()}
        onJumpToSource={jumpToSource}
      />
    </div>

    {#if settingsOpen}
      <div
        class="settings-popover"
        id="reader-settings"
        role="dialog"
        aria-labelledby="reader-settings-title"
        tabindex="-1"
        bind:this={settingsPanel}
      >
        <header>
          <strong id="reader-settings-title">Reading settings</strong>
          <button class="icon-button" onclick={() => void closeSettings()} aria-label="Close reading settings">Close</button>
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
      </div>
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
