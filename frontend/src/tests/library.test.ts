import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Library from '../components/Library.svelte';
import type { BookSummary } from '../lib/types';

const mocks = vi.hoisted(() => ({
  listBooks: vi.fn(),
  importBook: vi.fn(),
  getCover: vi.fn(),
  deleteBook: vi.fn(),
}));

vi.mock('../lib/commands', () => ({
  listBooks: mocks.listBooks,
  importBook: mocks.importBook,
  getCover: mocks.getCover,
  deleteBook: mocks.deleteBook,
  errorMessage: (error: unknown) => error instanceof Error ? error.message : String(error),
}));

const book: BookSummary = {
  id: 'book-1',
  title: 'The Test Book',
  author: 'Ada Reader',
  progress: { currentLocation: 25, currentChapterId: 'chapter-1', completionPercentage: 25 },
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => { resolve = resolvePromise; });
  return { promise, resolve };
}

describe('Library', () => {
  beforeEach(() => {
    mocks.importBook.mockResolvedValue(null);
    mocks.getCover.mockRejectedValue(new Error('no cover'));
    mocks.deleteBook.mockResolvedValue(true);
  });

  it('shows explicit loading and empty states', async () => {
    const pending = deferred<{ books: BookSummary[]; total: number }>();
    mocks.listBooks.mockReturnValue(pending.promise);
    render(Library, { onOpenBook: vi.fn() });

    expect(screen.getByText('Opening library')).toBeTruthy();
    pending.resolve({ books: [], total: 0 });

    expect(await screen.findByText('Bring one book. Start somewhere.')).toBeTruthy();
  });

  it('shows a recoverable core error and retries', async () => {
    mocks.listBooks
      .mockRejectedValueOnce(new Error('database locked'))
      .mockResolvedValueOnce({ books: [], total: 0 });
    render(Library, { onOpenBook: vi.fn() });

    expect(await screen.findByText('Your library could not be opened')).toBeTruthy();
    expect(screen.getByText('database locked')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Retry' }));

    expect(await screen.findByText('Bring one book. Start somewhere.')).toBeTruthy();
    expect(mocks.listBooks).toHaveBeenCalledTimes(2);
  });

  it('lets the native command choose a book and treats null as cancellation', async () => {
    const pendingImport = deferred<BookSummary>();
    mocks.listBooks.mockResolvedValue({ books: [], total: 0 });
    mocks.importBook.mockReturnValue(pendingImport.promise);
    render(Library, { onOpenBook: vi.fn() });

    await fireEvent.click(await screen.findByRole('button', { name: 'Import your first EPUB' }));
    expect(mocks.importBook).toHaveBeenCalledWith();
    expect(await screen.findByText('The reader core is opening or importing your book')).toBeTruthy();

    pendingImport.resolve(book);
    expect(await screen.findByText('The Test Book')).toBeTruthy();

    mocks.importBook.mockResolvedValueOnce(null);
    await fireEvent.click(screen.getByRole('button', { name: 'Import EPUB' }));
    expect(screen.getByText('The Test Book')).toBeTruthy();
  });

  it('searches, opens, and confirms deletion with semantic controls', async () => {
    const onOpenBook = vi.fn();
    mocks.listBooks.mockResolvedValue({ books: [book], total: 1 });
    render(Library, { onOpenBook });

    const readButton = await screen.findByRole('button', { name: 'Read The Test Book' });
    readButton.focus();
    await fireEvent.click(readButton);
    expect(onOpenBook).toHaveBeenCalledWith('book-1');

    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'missing' } });
    expect(screen.getByText('No matching books')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Clear search' }));

    await fireEvent.click(screen.getByRole('button', { name: 'Delete The Test Book' }));
    expect(screen.getByText('Remove this book?')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Remove' }));

    await waitFor(() => expect(mocks.deleteBook).toHaveBeenCalledWith('book-1'));
    expect(await screen.findByText('Bring one book. Start somewhere.')).toBeTruthy();
  });

  it('keeps the book when native deletion confirmation is cancelled', async () => {
    mocks.listBooks.mockResolvedValue({ books: [book], total: 1 });
    mocks.deleteBook.mockResolvedValue(false);
    render(Library, { onOpenBook: vi.fn() });

    await fireEvent.click(await screen.findByRole('button', { name: 'Delete The Test Book' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Remove' }));

    await waitFor(() => expect(mocks.deleteBook).toHaveBeenCalledWith('book-1'));
    expect(screen.getByRole('button', { name: 'Read The Test Book' })).toBeTruthy();
    expect(screen.queryByText('Remove this book?')).toBeNull();
  });
});
