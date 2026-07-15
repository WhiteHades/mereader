import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../App.svelte';
import type { BookDetail } from '../lib/types';

const mocks = vi.hoisted(() => ({
  listBooks: vi.fn(),
  getCover: vi.fn(),
  getBook: vi.fn(),
  getChapter: vi.fn(),
  updateProgress: vi.fn(),
  getAiStatus: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));
vi.mock('../lib/commands', () => ({
  ...mocks,
  importBook: vi.fn(),
  deleteBook: vi.fn(),
  askBook: vi.fn(),
  errorMessage: (error: unknown) => error instanceof Error ? error.message : String(error),
}));

const detail: BookDetail = {
  id: 'book-1',
  title: 'Navigation Book',
  author: 'Page Turner',
  progress: null,
  totalLocations: 10,
  chapters: [{ id: 'chapter-1', title: 'Start', order: 1, startLocation: 1, endLocation: 10 }],
};

describe('App navigation', () => {
  beforeEach(() => {
    mocks.listBooks.mockResolvedValue({ books: [detail], total: 1 });
    mocks.getCover.mockRejectedValue(new Error('no cover'));
    mocks.getBook.mockResolvedValue(detail);
    mocks.getChapter.mockResolvedValue({
      chapterId: 'chapter-1',
      title: 'Start',
      html: '<p>Reader destination</p>',
      startLocation: 1,
      endLocation: 10,
    });
    mocks.updateProgress.mockResolvedValue({ currentLocation: 1, currentChapterId: 'chapter-1', completionPercentage: 10 });
    mocks.getAiStatus.mockResolvedValue({ state: 'ready' });
  });

  it('switches between library and reader without a router', async () => {
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Read Navigation Book' }));
    expect(await screen.findByText('Reader destination')).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Library' }));
    expect(await screen.findByRole('button', { name: 'Read Navigation Book' })).toBeTruthy();
  });
});
