import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Reader from '../components/Reader.svelte';
import type { BookDetail } from '../lib/types';

const mocks = vi.hoisted(() => ({
  getBook: vi.fn(),
  getChapter: vi.fn(),
  getChapterAsset: vi.fn(),
  updateProgress: vi.fn(),
  getAiStatus: vi.fn(),
  askBook: vi.fn(),
}));

vi.mock('../lib/commands', () => ({
  getBook: mocks.getBook,
  getChapter: mocks.getChapter,
  getChapterAsset: mocks.getChapterAsset,
  updateProgress: mocks.updateProgress,
  getAiStatus: mocks.getAiStatus,
  askBook: mocks.askBook,
  errorMessage: (error: unknown) => error instanceof Error ? error.message : String(error),
}));

const detail: BookDetail = {
  id: 'book-1',
  title: 'A Reader Test',
  author: 'Test Author',
  totalLocations: 200,
  progress: {
    currentLocation: 40,
    currentChapterId: 'chapter-1',
    completionPercentage: 20,
  },
  chapters: [
    { id: 'chapter-1', title: 'Opening', order: 1, startLocation: 1, endLocation: 100 },
    { id: 'chapter-2', title: 'Ending', order: 2, startLocation: 101, endLocation: 200 },
  ],
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => { resolve = resolvePromise; });
  return { promise, resolve };
}

describe('Reader', () => {
  beforeEach(() => {
    mocks.getBook.mockResolvedValue(detail);
    mocks.getChapter.mockResolvedValue({
      chapterId: 'chapter-1',
      title: 'Opening',
      html: '<h1>Opening</h1><p>Sanitized chapter text.</p>',
      startLocation: 1,
      endLocation: 100,
    });
    mocks.updateProgress.mockResolvedValue(detail.progress);
    mocks.getChapterAsset.mockResolvedValue({ mimeType: 'image/png', data: [137, 80, 78, 71] });
    mocks.getAiStatus.mockResolvedValue({ state: 'ready' });
  });

  it('waits for book progress before loading and restoring chapter content', async () => {
    const pendingBook = deferred<BookDetail>();
    mocks.getBook.mockReturnValue(pendingBook.promise);
    render(Reader, { bookId: 'book-1', onBack: vi.fn() });

    expect(screen.getByText('Restoring your reading position')).toBeTruthy();
    expect(mocks.getChapter).not.toHaveBeenCalled();
    pendingBook.resolve(detail);

    expect(await screen.findByText('Sanitized chapter text.')).toBeTruthy();
    expect(mocks.getChapter).toHaveBeenCalledWith('book-1', 'chapter-1');
    expect(screen.getByLabelText('20% read')).toBeTruthy();
  });

  it('flushes an in-flight latest position before returning to the library', async () => {
    const pendingSave = deferred<typeof detail.progress>();
    const onBack = vi.fn();
    mocks.updateProgress.mockReturnValue(pendingSave.promise);
    render(Reader, { bookId: 'book-1', onBack });

    await screen.findByRole('region', { name: 'Opening' });
    await fireEvent.click(screen.getByRole('button', { name: 'Next chapter' }));
    await waitFor(() => expect(mocks.updateProgress).toHaveBeenCalledWith('book-1', 101, 'chapter-2'));

    await fireEvent.click(screen.getByRole('button', { name: 'Library' }));
    expect(onBack).not.toHaveBeenCalled();
    pendingSave.resolve(detail.progress);
    await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
  });

  it('closes open panels with Escape and permits only one narrow drawer', async () => {
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn((query: string) => ({
        matches: query.includes('max-width'),
        media: query,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    });
    render(Reader, { bookId: 'book-1', onBack: vi.fn() });
    await screen.findByText('Sanitized chapter text.');

    const contents = screen.getByRole('button', { name: 'Contents' });
    const askAi = screen.getByRole('button', { name: 'Ask AI' });
    await fireEvent.click(contents);
    expect(contents.getAttribute('aria-expanded')).toBe('true');
    await fireEvent.click(askAi);
    expect(contents.getAttribute('aria-expanded')).toBe('false');
    expect(askAi.getAttribute('aria-expanded')).toBe('true');

    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(askAi.getAttribute('aria-expanded')).toBe('false');
  });

  it('shows a recoverable book load error', async () => {
    mocks.getBook.mockRejectedValueOnce(new Error('book database unavailable')).mockResolvedValueOnce(detail);
    render(Reader, { bookId: 'book-1', onBack: vi.fn() });

    expect(await screen.findByText('book database unavailable')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(await screen.findByText('Sanitized chapter text.')).toBeTruthy();
  });

  it('assigns an adjacent chapter boundary to the following chapter', async () => {
    mocks.getBook.mockResolvedValue({
      ...detail,
      progress: {
        currentLocation: 100,
        currentChapterId: 'chapter-1',
        completionPercentage: 50,
      },
      chapters: [
        { id: 'chapter-1', title: 'Opening', order: 1, startLocation: 0, endLocation: 100 },
        { id: 'chapter-2', title: 'Ending', order: 2, startLocation: 100, endLocation: 200 },
      ],
    });
    mocks.getChapter.mockResolvedValue({
      chapterId: 'chapter-2',
      title: 'Ending',
      html: '<p>Boundary chapter.</p>',
      startLocation: 100,
      endLocation: 200,
    });

    render(Reader, { bookId: 'book-1', onBack: vi.fn() });

    expect(await screen.findByText('Boundary chapter.')).toBeTruthy();
    expect(mocks.getChapter).toHaveBeenCalledWith('book-1', 'chapter-2');
  });

  it('loads only generated chapter assets and revokes object URLs', async () => {
    const firstAsset = '11111111-1111-4111-8111-111111111111.png';
    const secondAsset = '22222222-2222-4222-8222-222222222222.webp';
    mocks.getChapter.mockImplementation((_bookId: string, chapterId: string) => Promise.resolve({
      chapterId,
      title: chapterId === 'chapter-1' ? 'Opening' : 'Ending',
      html: chapterId === 'chapter-1'
        ? `<p>First image</p><img alt="valid" src="assets/${firstAsset}"><img alt="invalid" src="assets/../secret.png">`
        : `<p>Second image</p><img alt="second" src="assets/${secondAsset}">`,
      startLocation: chapterId === 'chapter-1' ? 1 : 101,
      endLocation: chapterId === 'chapter-1' ? 100 : 200,
    }));
    vi.mocked(URL.createObjectURL)
      .mockReturnValueOnce('blob:first')
      .mockReturnValueOnce('blob:second');

    const view = render(Reader, { bookId: 'book-1', onBack: vi.fn() });

    expect(await screen.findByText('First image')).toBeTruthy();
    expect(screen.getByAltText('valid').getAttribute('src')).toBe('blob:first');
    expect(screen.getByAltText('invalid').hasAttribute('src')).toBe(false);
    expect(mocks.getChapterAsset).toHaveBeenCalledTimes(1);
    expect(mocks.getChapterAsset).toHaveBeenCalledWith('book-1', firstAsset);

    await fireEvent.click(screen.getByRole('button', { name: 'Next chapter' }));
    expect(await screen.findByText('Second image')).toBeTruthy();
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:first');

    view.unmount();
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:second');
  });
});
