import { describe, expect, it } from 'vitest';
import {
  chapterContainsLocation,
  findChapterAtLocation,
} from '../lib/chapters';
import type { ChapterSummary } from '../lib/types';

const chapters: ChapterSummary[] = [
  { id: 'image', title: 'Image', order: 0, startLocation: 0, endLocation: 1 },
  { id: 'first', title: 'First', order: 1, startLocation: 1, endLocation: 11 },
  { id: 'empty', title: 'Empty', order: 2, startLocation: 11, endLocation: 12 },
  { id: 'last', title: 'Last', order: 3, startLocation: 12, endLocation: 22 },
];

describe('chapter boundaries', () => {
  it('keeps ordinary chapter lookup half-open', () => {
    expect(chapterContainsLocation(chapters, chapters[1], 10)).toBe(true);
    expect(chapterContainsLocation(chapters, chapters[1], 11)).toBe(false);
    expect(findChapterAtLocation(chapters, 12)?.id).toBe('last');
    expect(chapterContainsLocation(chapters, chapters[3], 22)).toBe(true);
  });

  it('keeps the reserved location for each zero-text chapter addressable', () => {
    expect(chapterContainsLocation(chapters, chapters[0], 0)).toBe(true);
    expect(chapterContainsLocation(chapters, chapters[2], 11)).toBe(true);
  });
});
