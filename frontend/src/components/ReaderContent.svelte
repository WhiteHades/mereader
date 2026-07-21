<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { errorMessage, getChapter, getChapterAsset } from '../lib/commands';
  import {
    chapterContainsLocation,
    findChapterAtLocation,
    lastLocationInChapter,
  } from '../lib/chapters';
  import {
    createTextAnchors,
    lastFullyVisibleLocation,
    nearestTextAnchor,
    type TextAnchor,
  } from '../lib/text-anchors';
  import type { ChapterContent, ChapterSummary, ReaderSettings } from '../lib/types';

  interface PositionChange {
    location: number;
    chapterId: string;
  }

  interface Props {
    bookId: string;
    chapters: ChapterSummary[];
    initialLocation: number;
    initialChapterId: string | null;
    settings: ReaderSettings;
    onPositionChange: (position: PositionChange) => void;
    onChapterChange: (chapter: ChapterSummary) => void;
  }

  let {
    bookId,
    chapters,
    initialLocation,
    initialChapterId,
    settings,
    onPositionChange,
    onChapterChange,
  }: Props = $props();

  let scroller = $state<HTMLElement>();
  let contentRoot = $state<HTMLElement>();
  let chapterContent = $state<ChapterContent | null>(null);
  let currentChapter = $state<ChapterSummary | null>(null);
  let currentLocation = $state(0);
  let loading = $state(true);
  let chapterError = $state('');
  let restoring = false;
  let scrollFrame = 0;
  let loadVersion = 0;
  let destroyed = false;
  let chapterAssetUrls: string[] = [];
  let textAnchors: TextAnchor[] = [];
  let userScrolled = false;
  let previousSettingsSignature: string | null = null;

  const generatedAssetPattern = /^assets\/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.(?:jpg|png|gif|webp))$/;

  onMount(() => {
    currentLocation = initialLocation;
    const initialChapter = chapters.find((chapter) =>
      chapter.id === initialChapterId &&
      chapterContainsLocation(chapters, chapter, initialLocation)
    );
    if (initialChapter) {
      void loadChapter(initialChapter, initialLocation, false);
    } else {
      void navigateToLocation(initialLocation, false);
    }
    return () => {
      destroyed = true;
      loadVersion += 1;
      cancelAnimationFrame(scrollFrame);
      revokeChapterAssets();
      textAnchors = [];
    };
  });

  $effect(() => {
    const signature = settingsSignature(settings);
    if (previousSettingsSignature === null) {
      previousSettingsSignature = signature;
      return;
    }
    if (signature === previousSettingsSignature) return;
    previousSettingsSignature = signature;
    void rebuildAfterSettingsChange();
  });

  export async function goToLocation(location: number): Promise<void> {
    await navigateToLocation(location, true);
  }

  export function focusContent(): void {
    scroller?.focus({ preventScroll: true });
  }

  export function flushPosition(): void {
    if (!scrollFrame) return;
    cancelAnimationFrame(scrollFrame);
    scrollFrame = 0;
    reportVisiblePosition();
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
    textAnchors = [];
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
      userScrolled = false;
      onChapterChange(chapter);
      await tick();
      rebuildTextAnchors();
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
      const images = [...document.querySelectorAll<HTMLImageElement>('img[src]')];
      for (let offset = 0; offset < images.length; offset += 4) {
        await Promise.all(images.slice(offset, offset + 4).map(async (image) => {
          const source = image.getAttribute('src') ?? '';
          const match = generatedAssetPattern.exec(source);
          if (!match) {
            image.removeAttribute('src');
            return;
          }
          try {
            const asset = await getChapterAsset(bookId, match[1]);
            const objectUrl = URL.createObjectURL(
              new Blob([Uint8Array.from(asset.data)], { type: asset.mimeType }),
            );
            objectUrls.push(objectUrl);
            image.setAttribute('src', objectUrl);
          } catch {
            image.removeAttribute('src');
          }
        }));
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

  function rebuildTextAnchors(): void {
    textAnchors = contentRoot && currentChapter
      ? createTextAnchors(contentRoot, currentChapter.startLocation)
      : [];
  }

  async function rebuildAfterSettingsChange(): Promise<void> {
    const location = currentLocation;
    await tick();
    if (!contentRoot || !currentChapter) return;
    rebuildTextAnchors();
    scrollToLocation(location, false);
  }

  function watchChapterAssets(node: HTMLElement): { destroy: () => void } {
    const handleAssetSettled = (event: Event): void => {
      if (!(event.target instanceof HTMLImageElement)) return;
      const location = currentLocation;
      requestAnimationFrame(() => {
        if (node !== contentRoot || !currentChapter) return;
        rebuildTextAnchors();
        if (!userScrolled) scrollToLocation(location, false);
      });
    };
    node.addEventListener('load', handleAssetSettled, true);
    node.addEventListener('error', handleAssetSettled, true);
    return {
      destroy: () => {
        node.removeEventListener('load', handleAssetSettled, true);
        node.removeEventListener('error', handleAssetSettled, true);
      },
    };
  }

  function scrollToLocation(location: number, reportPosition: boolean): void {
    if (!scroller || !currentChapter) return;
    restoring = true;
    const bounded = Math.max(
      currentChapter.startLocation,
      Math.min(currentChapter.endLocation, location),
    );
    if (bounded <= currentChapter.startLocation || textAnchors.length === 0) {
      scroller.scrollTop = 0;
    } else {
      const anchor = nearestTextAnchor(textAnchors, bounded);
      if (anchor && typeof anchor.range.getBoundingClientRect === 'function') {
        const anchorRect = anchor.range.getBoundingClientRect();
        const viewport = scroller.getBoundingClientRect();
        const maxScroll = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
        const target = scroller.scrollTop + anchorRect.top - viewport.top;
        scroller.scrollTop = Math.max(0, Math.min(maxScroll, target));
      }
    }
    currentLocation = bounded;
    if (reportPosition) {
      onPositionChange({ location: bounded, chapterId: currentChapter.id });
    }
    requestAnimationFrame(() => {
      restoring = false;
    });
  }

  function handleScroll(): void {
    if (restoring || !scroller || !currentChapter) return;
    userScrolled = true;
    cancelAnimationFrame(scrollFrame);
    scrollFrame = requestAnimationFrame(() => {
      scrollFrame = 0;
      reportVisiblePosition();
    });
  }

  function reportVisiblePosition(): void {
    if (!scroller || !currentChapter) return;
    const location = lastFullyVisibleLocation(
      textAnchors,
      scroller.getBoundingClientRect(),
      currentChapter.startLocation,
      currentChapter.endLocation,
    );
    if (location === currentLocation) return;
    currentLocation = location;
    const index = chapterIndex();
    const progressChapter = location === currentChapter.endLocation &&
      currentChapter.startLocation < currentChapter.endLocation &&
      index >= 0 && index < chapters.length - 1
      ? chapters[index + 1]
      : currentChapter;
    onPositionChange({ location, chapterId: progressChapter.id });
  }

  function chapterIndex(): number {
    return chapters.findIndex((chapter) => chapter.id === currentChapter?.id);
  }

  function scrollBehavior(): ScrollBehavior {
    return window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth';
  }

  function settingsSignature(value: ReaderSettings): string {
    return `${value.fontSize}:${value.lineHeight}:${value.readingWidth}`;
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
      tabindex="-1"
      style={`--reader-font-size: ${settings.fontSize}px; --reader-line-height: ${settings.lineHeight}; --reader-width: ${settings.readingWidth}px`}
    >
      <div
        class="chapter-content"
        data-testid="chapter-content"
        bind:this={contentRoot}
        use:watchChapterAssets
      >
        <!-- Rust sanitizes markup; the frontend replaces validated generated image names with Blob URLs. -->
        {@html chapterContent.html}
      </div>
    </div>
  {/if}
</section>
