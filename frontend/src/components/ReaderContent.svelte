<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { errorMessage, getChapter, getChapterAsset } from '../lib/commands';
  import { findChapterAtLocation, lastLocationInChapter } from '../lib/chapters';
  import type { ChapterContent, ChapterSummary, ReaderSettings } from '../lib/types';

  interface PositionChange {
    location: number;
    chapterId: string;
  }

  interface Props {
    bookId: string;
    chapters: ChapterSummary[];
    initialLocation: number;
    settings: ReaderSettings;
    onPositionChange: (position: PositionChange) => void;
    onChapterChange: (chapter: ChapterSummary) => void;
  }

  let {
    bookId,
    chapters,
    initialLocation,
    settings,
    onPositionChange,
    onChapterChange,
  }: Props = $props();

  let scroller = $state<HTMLElement>();
  let chapterContent = $state<ChapterContent | null>(null);
  let currentChapter = $state<ChapterSummary | null>(null);
  let currentLocation = $state(1);
  let loading = $state(true);
  let chapterError = $state('');
  let restoring = false;
  let scrollFrame = 0;
  let loadVersion = 0;
  let destroyed = false;
  let chapterAssetUrls: string[] = [];

  const generatedAssetPattern = /^assets\/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.(?:jpg|png|gif|webp|avif))$/;

  onMount(() => {
    currentLocation = initialLocation;
    void navigateToLocation(initialLocation, false);
    return () => {
      destroyed = true;
      loadVersion += 1;
      cancelAnimationFrame(scrollFrame);
      revokeChapterAssets();
    };
  });

  export async function goToLocation(location: number): Promise<void> {
    await navigateToLocation(location, true);
  }

  async function navigateToLocation(location: number, reportPosition: boolean): Promise<void> {
    const target = findChapterAtLocation(chapters, location) ?? chapters[0];
    if (!target) {
      chapterError = 'This book does not contain any readable chapters.';
      loading = false;
      return;
    }

    const bounded = Math.max(
      target.startLocation,
      Math.min(lastLocationInChapter(chapters, target), location),
    );
    if (currentChapter?.id === target.id && chapterContent) {
      scrollToLocation(bounded, reportPosition);
      return;
    }
    await loadChapter(target, bounded, reportPosition);
  }

  export async function goToChapter(chapterId: string): Promise<void> {
    const chapter = chapters.find((entry) => entry.id === chapterId);
    if (chapter) await loadChapter(chapter, chapter.startLocation, true);
  }

  export async function previousChapter(): Promise<void> {
    const index = chapterIndex();
    if (index > 0) await loadChapter(chapters[index - 1], chapters[index - 1].startLocation, true);
  }

  export async function nextChapter(): Promise<void> {
    const index = chapterIndex();
    if (index >= 0 && index < chapters.length - 1) {
      await loadChapter(chapters[index + 1], chapters[index + 1].startLocation, true);
    }
  }

  export async function previousViewport(): Promise<void> {
    if (!scroller) return;
    if (scroller.scrollTop <= 1 && chapterIndex() > 0) {
      const previous = chapters[chapterIndex() - 1];
      await loadChapter(previous, lastLocationInChapter(chapters, previous), true);
      return;
    }
    scroller.scrollBy({ top: -scroller.clientHeight * 0.82, behavior: scrollBehavior() });
  }

  export async function nextViewport(): Promise<void> {
    if (!scroller) return;
    const atEnd = scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 2;
    if (atEnd && chapterIndex() < chapters.length - 1) {
      await nextChapter();
      return;
    }
    scroller.scrollBy({ top: scroller.clientHeight * 0.82, behavior: scrollBehavior() });
  }

  async function loadChapter(
    chapter: ChapterSummary,
    location: number,
    reportPosition: boolean,
  ): Promise<void> {
    const version = ++loadVersion;
    revokeChapterAssets();
    chapterContent = null;
    loading = true;
    chapterError = '';
    try {
      const loaded = await getChapter(bookId, chapter.id);
      const resolved = await resolveChapterAssets(loaded.html);
      if (destroyed || version !== loadVersion) {
        resolved.objectUrls.forEach((url) => URL.revokeObjectURL(url));
        return;
      }
      chapterAssetUrls = resolved.objectUrls;
      chapterContent = { ...loaded, html: resolved.html };
      currentChapter = chapter;
      currentLocation = location;
      onChapterChange(chapter);
      await tick();
      scrollToLocation(location, reportPosition);
    } catch (error) {
      if (!destroyed && version === loadVersion) chapterError = errorMessage(error);
    } finally {
      if (!destroyed && version === loadVersion) loading = false;
    }
  }

  async function resolveChapterAssets(html: string): Promise<{ html: string; objectUrls: string[] }> {
    const document = new DOMParser().parseFromString(html, 'text/html');
    const objectUrls: string[] = [];
    try {
      for (const image of document.querySelectorAll<HTMLImageElement>('img[src]')) {
        const source = image.getAttribute('src') ?? '';
        const match = generatedAssetPattern.exec(source);
        if (!match) {
          image.removeAttribute('src');
          continue;
        }
        const asset = await getChapterAsset(bookId, match[1]);
        const objectUrl = URL.createObjectURL(
          new Blob([Uint8Array.from(asset.data)], { type: asset.mimeType }),
        );
        objectUrls.push(objectUrl);
        image.setAttribute('src', objectUrl);
      }
      return { html: document.body.innerHTML, objectUrls };
    } catch (error) {
      objectUrls.forEach((url) => URL.revokeObjectURL(url));
      throw error;
    }
  }

  function revokeChapterAssets(): void {
    chapterAssetUrls.forEach((url) => URL.revokeObjectURL(url));
    chapterAssetUrls = [];
  }

  function scrollToLocation(location: number, reportPosition: boolean): void {
    if (!scroller || !currentChapter) return;
    restoring = true;
    const maximum = lastLocationInChapter(chapters, currentChapter);
    const range = Math.max(1, maximum - currentChapter.startLocation);
    const fraction = (location - currentChapter.startLocation) / range;
    const maxScroll = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
    scroller.scrollTop = Math.max(0, Math.min(maxScroll, fraction * maxScroll));
    currentLocation = location;
    if (reportPosition) {
      onPositionChange({ location, chapterId: currentChapter.id });
    }
    requestAnimationFrame(() => {
      restoring = false;
    });
  }

  function handleScroll(): void {
    if (restoring || !scroller || !currentChapter) return;
    cancelAnimationFrame(scrollFrame);
    scrollFrame = requestAnimationFrame(() => {
      if (!scroller || !currentChapter) return;
      const maxScroll = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
      const fraction = maxScroll === 0 ? 0 : scroller.scrollTop / maxScroll;
      const maximum = lastLocationInChapter(chapters, currentChapter);
      const range = maximum - currentChapter.startLocation;
      const location = Math.max(
        currentChapter.startLocation,
        Math.min(maximum, Math.round(currentChapter.startLocation + range * fraction)),
      );
      if (location === currentLocation) return;
      currentLocation = location;
      onPositionChange({ location, chapterId: currentChapter.id });
    });
  }

  function chapterIndex(): number {
    return chapters.findIndex((chapter) => chapter.id === currentChapter?.id);
  }

  function scrollBehavior(): ScrollBehavior {
    return window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth';
  }
</script>

<section class="reader-content-shell" aria-label="Book content">
  {#if loading}
    <div class="reader-state" role="status" aria-live="polite">
      <span class="activity-dot" aria-hidden="true"></span>
      <p>Preparing chapter</p>
    </div>
  {/if}

  {#if chapterError}
    <div class="reader-state" role="alert">
      <p class="state-code">CORE / CHAPTER</p>
      <h2>Chapter unavailable</h2>
      <p>{chapterError}</p>
      <button class="button primary" onclick={() => navigateToLocation(currentLocation, false)}>Retry chapter</button>
    </div>
  {:else if chapterContent}
    <div
      class="chapter-scroller"
      bind:this={scroller}
      onscroll={handleScroll}
      role="region"
      aria-label={chapterContent.title}
      style={`--reader-font-size: ${settings.fontSize}px; --reader-line-height: ${settings.lineHeight}; --reader-width: ${settings.readingWidth}px`}
    >
      <div class="chapter-content" data-testid="chapter-content">
        <!-- Rust sanitizes markup; the frontend replaces validated generated image names with Blob URLs. -->
        {@html chapterContent.html}
      </div>
    </div>
  {/if}
</section>
