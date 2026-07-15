import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ReaderContent from '../components/ReaderContent.svelte';
import type { ChapterSummary, ReaderSettings } from '../lib/types';

const mocks = vi.hoisted(() => ({
  getChapter: vi.fn(),
  getChapterAsset: vi.fn(),
}));

vi.mock('../lib/commands', () => ({
  getChapter: mocks.getChapter,
  getChapterAsset: mocks.getChapterAsset,
  errorMessage: (error: unknown) => error instanceof Error ? error.message : String(error),
}));

const assetName = '11111111-1111-4111-8111-111111111111.png';
const settings: ReaderSettings = {
  theme: 'sepia',
  fontSize: 19,
  lineHeight: 1.7,
  readingWidth: 700,
};

function domRect(top: number, bottom: number): DOMRect {
  return {
    top,
    bottom,
    left: 0,
    right: 500,
    width: 500,
    height: bottom - top,
    x: 0,
    y: top,
    toJSON: () => ({}),
  };
}

describe('ReaderContent text-position tracking', () => {
  beforeEach(() => {
    mocks.getChapterAsset.mockResolvedValue({ mimeType: 'image/png', data: [137, 80, 78, 71] });
  });

  it('does not advance into text below a tall leading image', async () => {
    const chapters: ChapterSummary[] = [
      { id: 'chapter-1', title: 'Illustrated', order: 0, startLocation: 0, endLocation: 11 },
    ];
    const onPositionChange = vi.fn();
    mocks.getChapter.mockResolvedValue({
      chapterId: 'chapter-1',
      title: 'Illustrated',
      html: `<img alt="Frontispiece" src="assets/${assetName}"><p>Later words</p>`,
      startLocation: 0,
      endLocation: 11,
    });
    vi.spyOn(Range.prototype, 'getClientRects').mockReturnValue([
      domRect(900, 920),
    ] as unknown as DOMRectList);

    render(ReaderContent, {
      bookId: 'book-1',
      chapters,
      initialLocation: 0,
      initialChapterId: 'chapter-1',
      settings,
      onPositionChange,
      onChapterChange: vi.fn(),
    });

    const scroller = await screen.findByRole('region', { name: 'Illustrated' });
    vi.spyOn(scroller, 'getBoundingClientRect').mockReturnValue(domRect(0, 500));
    Object.defineProperties(scroller, {
      clientHeight: { configurable: true, value: 500 },
      scrollHeight: { configurable: true, value: 1500 },
    });
    scroller.scrollTop = 400;
    await fireEvent.scroll(scroller);
    await new Promise((resolve) => window.setTimeout(resolve, 10));

    expect(onPositionChange).not.toHaveBeenCalled();
  });

  it('uses the following chapter ID when visible text reaches a half-open boundary', async () => {
    const chapters: ChapterSummary[] = [
      { id: 'chapter-1', title: 'First', order: 0, startLocation: 0, endLocation: 11 },
      { id: 'chapter-2', title: 'Second', order: 1, startLocation: 11, endLocation: 15 },
    ];
    const onPositionChange = vi.fn();
    mocks.getChapter.mockResolvedValue({
      chapterId: 'chapter-1',
      title: 'First',
      html: '<p>Final words</p>',
      startLocation: 0,
      endLocation: 11,
    });
    vi.spyOn(Range.prototype, 'getClientRects').mockReturnValue([
      domRect(20, 40),
    ] as unknown as DOMRectList);

    render(ReaderContent, {
      bookId: 'book-1',
      chapters,
      initialLocation: 0,
      initialChapterId: 'chapter-1',
      settings,
      onPositionChange,
      onChapterChange: vi.fn(),
    });

    const scroller = await screen.findByRole('region', { name: 'First' });
    vi.spyOn(scroller, 'getBoundingClientRect').mockReturnValue(domRect(0, 500));
    await fireEvent.scroll(scroller);

    await waitFor(() => expect(onPositionChange).toHaveBeenCalledWith({
      location: 11,
      chapterId: 'chapter-2',
    }));
    await fireEvent.scroll(scroller);
    await new Promise((resolve) => window.setTimeout(resolve, 10));
    expect(onPositionChange).toHaveBeenCalledTimes(1);
  });
});
