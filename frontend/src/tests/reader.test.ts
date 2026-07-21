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
  cancelAiRequest: vi.fn(),
  reindexBook: vi.fn(),
  onCloseRequested: vi.fn(),
  destroyWindow: vi.fn(),
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onCloseRequested: mocks.onCloseRequested,
    destroy: mocks.destroyWindow,
  }),
}));

vi.mock('../lib/commands', () => ({
  getBook: mocks.getBook,
  getChapter: mocks.getChapter,
  getChapterAsset: mocks.getChapterAsset,
  updateProgress: mocks.updateProgress,
  getAiStatus: mocks.getAiStatus,
  askBook: mocks.askBook,
  cancelAiRequest: mocks.cancelAiRequest,
  reindexBook: mocks.reindexBook,
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
    { id: 'chapter-1', title: 'Opening', order: 0, startLocation: 0, endLocation: 100 },
    { id: 'chapter-2', title: 'Ending', order: 1, startLocation: 100, endLocation: 200 },
  ],
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => { resolve = resolvePromise; });
  return { promise, resolve };
}

describe('Reader', () => {
  beforeEach(() => {
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn((query: string) => ({
        matches: false,
        media: query,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    });
    mocks.getBook.mockResolvedValue(detail);
    mocks.getChapter.mockResolvedValue({
      chapterId: 'chapter-1',
      title: 'Opening',
      html: '<h1>Opening</h1><p>Sanitized chapter text.</p>',
      startLocation: 0,
      endLocation: 100,
    });
    mocks.updateProgress.mockResolvedValue(detail.progress);
    mocks.getChapterAsset.mockResolvedValue({ mimeType: 'image/png', data: [137, 80, 78, 71] });
    mocks.getAiStatus.mockResolvedValue({ state: 'ready' });
    mocks.reindexBook.mockResolvedValue({ state: 'ready' });
    mocks.askBook.mockResolvedValue({ answer: '', sources: [], progressBoundary: null });
    mocks.cancelAiRequest.mockResolvedValue(true);
    mocks.onCloseRequested.mockResolvedValue(() => {});
    mocks.destroyWindow.mockResolvedValue(undefined);
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
    await waitFor(() => expect(mocks.updateProgress).toHaveBeenCalledWith('book-1', 100, 'chapter-2'));

    await fireEvent.click(screen.getByRole('button', { name: 'Library' }));
    expect(onBack).not.toHaveBeenCalled();
    pendingSave.resolve(detail.progress);
    await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
  });

  it('prevents native close until an in-flight position save finishes', async () => {
    const pendingSave = deferred<typeof detail.progress>();
    let closeHandler: ((event: { preventDefault: () => void }) => Promise<void>) | undefined;
    mocks.onCloseRequested.mockImplementation(async (handler) => {
      closeHandler = handler;
      return () => {};
    });
    mocks.updateProgress.mockReturnValue(pendingSave.promise);
    render(Reader, { bookId: 'book-1', onBack: vi.fn() });

    await screen.findByRole('region', { name: 'Opening' });
    await waitFor(() => expect(closeHandler).toBeTypeOf('function'));
    await fireEvent.click(screen.getByRole('button', { name: 'Next chapter' }));
    await waitFor(() => expect(mocks.updateProgress).toHaveBeenCalled());
    const preventDefault = vi.fn();
    const closing = closeHandler!({ preventDefault });

    expect(preventDefault).toHaveBeenCalledTimes(1);
    expect(mocks.destroyWindow).not.toHaveBeenCalled();
    pendingSave.resolve(detail.progress);
    await closing;
    expect(mocks.destroyWindow).toHaveBeenCalledTimes(1);
  });

  it('restores chapter and viewport keyboard navigation outside controls', async () => {
    mocks.getChapter.mockImplementation(async (_bookId: string, chapterId: string) => ({
      chapterId,
      title: chapterId === 'chapter-1' ? 'Opening' : 'Ending',
      html: `<p>${chapterId}</p>`,
      startLocation: chapterId === 'chapter-1' ? 0 : 100,
      endLocation: chapterId === 'chapter-1' ? 100 : 200,
    }));
    render(Reader, { bookId: 'book-1', onBack: vi.fn() });
    await screen.findByRole('region', { name: 'Opening' });

    await fireEvent.keyDown(window, { key: 'ArrowRight' });

    await waitFor(() => expect(mocks.getChapter).toHaveBeenCalledWith('book-1', 'chapter-2'));
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
    expect(screen.getByRole('dialog', { name: 'Ask MeReader' }).getAttribute('aria-modal')).toBe('true');

    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(askAi.getAttribute('aria-expanded')).toBe('false');
    expect(document.activeElement).toBe(askAi);
  });

  it('restores trigger focus on close and labels drawers as dialogs', async () => {
    render(Reader, { bookId: 'book-1', onBack: vi.fn() });
    await screen.findByText('Sanitized chapter text.');

    const contents = screen.getByRole('button', { name: 'Contents' });
    await fireEvent.click(contents);
    const dialog = screen.getByRole('dialog', { name: 'Contents' });
    expect(dialog).toBeTruthy();
    expect(dialog.hasAttribute('aria-modal')).toBe(false);
    await fireEvent.click(screen.getByRole('button', { name: 'Close contents panel' }));
    expect(document.activeElement).toBe(contents);

    const askAi = screen.getByRole('button', { name: 'Ask AI' });
    await fireEvent.click(askAi);
    await fireEvent.click(screen.getByRole('button', { name: 'Close AI panel' }));
    expect(document.activeElement).toBe(askAi);
    const hiddenAiPanel = document.getElementById('ai-panel');
    expect(hiddenAiPanel?.hasAttribute('hidden')).toBe(true);
    expect(hiddenAiPanel?.inert).toBe(true);

    const text = screen.getByRole('button', { name: 'Text' });
    await fireEvent.click(text);
    await fireEvent.click(screen.getByRole('button', { name: 'Close reading settings' }));
    expect(document.activeElement).toBe(text);
  });

  it('uses one-based TOC labels for zero-based chapter order', async () => {
    render(Reader, { bookId: 'book-1', onBack: vi.fn() });
    await screen.findByText('Sanitized chapter text.');
    await fireEvent.click(screen.getByRole('button', { name: 'Contents' }));

    expect(screen.getByText('01')).toBeTruthy();
    expect(screen.getByText('02')).toBeTruthy();
  });

  it('closes AI and focuses reader content after a citation jump', async () => {
    mocks.askBook.mockResolvedValue({
      answer: 'Grounded answer [S1].',
      progressBoundary: null,
      sources: [{
        citationId: 'S1',
        chapterId: 'chapter-1',
        chapterTitle: 'Opening',
        text: 'Source text',
        startLocation: 10,
        endLocation: 20,
        relevanceScore: 0.1,
        retrievalMethods: ['keyword'],
      }],
    });
    render(Reader, { bookId: 'book-1', onBack: vi.fn() });
    await screen.findByText('Sanitized chapter text.');
    const askAi = screen.getByRole('button', { name: 'Ask AI' });
    await fireEvent.click(askAi);
    const input = await screen.findByLabelText('Question about this book');
    await fireEvent.input(input, { target: { value: 'What is grounded?' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Ask' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Jump to source S1' }));

    await waitFor(() => expect(askAi.getAttribute('aria-expanded')).toBe('false'));
    await waitFor(() => expect(document.activeElement).toBe(
      screen.getByRole('region', { name: 'Opening' }),
    ));
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

  it('saves image-only chapter starts and next chapter navigation', async () => {
    const imageOnlyDetail: BookDetail = {
      ...detail,
      progress: {
        currentLocation: 1,
        currentChapterId: 'chapter-2',
        completionPercentage: 5,
      },
      chapters: [
        { id: 'chapter-image', title: 'Plate', order: 0, startLocation: 0, endLocation: 1 },
        { id: 'chapter-2', title: 'Text', order: 1, startLocation: 1, endLocation: 20 },
      ],
      totalLocations: 20,
    };
    mocks.getBook.mockResolvedValue(imageOnlyDetail);
    mocks.getChapter.mockImplementation((_bookId: string, chapterId: string) => Promise.resolve({
      chapterId,
      title: chapterId === 'chapter-image' ? 'Plate' : 'Text',
      html: chapterId === 'chapter-image' ? '<img alt="A plate">' : '<p>Text chapter</p>',
      startLocation: chapterId === 'chapter-image' ? 0 : 1,
      endLocation: chapterId === 'chapter-image' ? 1 : 20,
    }));
    render(Reader, { bookId: 'book-1', onBack: vi.fn() });

    await screen.findByText('Text chapter');
    await fireEvent.click(screen.getByRole('button', { name: 'Previous chapter' }));
    expect(await screen.findByAltText('A plate')).toBeTruthy();
    await waitFor(() => expect(mocks.updateProgress).toHaveBeenCalledWith('book-1', 0, 'chapter-image'));

    await fireEvent.click(screen.getByRole('button', { name: 'Next chapter' }));
    expect(await screen.findByText('Text chapter')).toBeTruthy();
    await waitFor(() => expect(mocks.updateProgress).toHaveBeenCalledWith('book-1', 1, 'chapter-2'));
  });
});
