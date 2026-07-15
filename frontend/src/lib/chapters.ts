import type { ChapterSummary } from './types';

export function chapterContainsLocation(
  chapters: ChapterSummary[],
  chapter: ChapterSummary,
  location: number,
): boolean {
  const index = chapters.findIndex((entry) => entry.id === chapter.id);
  return location >= chapter.startLocation && (
    location < chapter.endLocation ||
    (index === chapters.length - 1 && location <= chapter.endLocation)
  );
}

export function findChapterAtLocation(
  chapters: ChapterSummary[],
  location: number,
): ChapterSummary | undefined {
  return chapters.find((chapter) => chapterContainsLocation(chapters, chapter, location));
}

export function lastLocationInChapter(
  chapters: ChapterSummary[],
  chapter: ChapterSummary,
): number {
  return chapters.at(-1)?.id === chapter.id
    ? chapter.endLocation
    : Math.max(chapter.startLocation, chapter.endLocation - 1);
}
