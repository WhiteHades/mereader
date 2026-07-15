import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AiEvent } from '../lib/types';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  channels: [] as Array<{ onmessage: (event: AiEvent) => void }>,
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: mocks.invoke,
  Channel: class MockChannel {
    onmessage = (_event: AiEvent): void => {};

    constructor() {
      mocks.channels.push(this);
    }
  },
}));

import {
  askBook,
  deleteBook,
  errorMessage,
  getAiStatus,
  getBook,
  getChapter,
  getChapterAsset,
  getCover,
  importBook,
  listBooks,
  reindexBook,
  updateProgress,
} from '../lib/commands';

describe('Tauri command wrappers', () => {
  beforeEach(() => {
    mocks.invoke.mockResolvedValue(undefined);
    mocks.channels.length = 0;
  });

  it('uses exact command names and camelCase arguments', async () => {
    await listBooks();
    await importBook();
    await getBook('book-1');
    await getCover('book-1');
    await getChapter('book-1', 'chapter-2');
    await getChapterAsset('book-1', '11111111-1111-4111-8111-111111111111.png');
    await updateProgress('book-1', 42, 'chapter-2');
    await deleteBook('book-1');
    await getAiStatus('book-1');
    await reindexBook('book-1');

    expect(mocks.invoke.mock.calls).toEqual([
      ['list_books'],
      ['import_book'],
      ['get_book', { bookId: 'book-1' }],
      ['get_cover', { bookId: 'book-1' }],
      ['get_chapter', { bookId: 'book-1', chapterId: 'chapter-2' }],
      ['get_chapter_asset', {
        bookId: 'book-1',
        assetName: '11111111-1111-4111-8111-111111111111.png',
      }],
      ['update_progress', { bookId: 'book-1', currentLocation: 42, currentChapterId: 'chapter-2' }],
      ['delete_book', { bookId: 'book-1' }],
      ['get_ai_status', { bookId: 'book-1' }],
      ['reindex_book', { bookId: 'book-1' }],
    ]);
  });

  it('passes a typed channel to ask_book', async () => {
    const response = { answer: 'Answer', sources: [], progressBoundary: null };
    const onEvent = vi.fn();
    mocks.invoke.mockResolvedValue(response);

    await expect(askBook('book-1', 'Why?', onEvent)).resolves.toBe(response);
    const channel = mocks.channels[0];
    const event: AiEvent = { type: 'delta', delta: 'Part' };
    channel.onmessage(event);

    expect(onEvent).toHaveBeenCalledWith(event);
    expect(mocks.invoke).toHaveBeenCalledWith('ask_book', {
      bookId: 'book-1',
      question: 'Why?',
      onEvent: channel,
    });
  });

  it('normalizes core errors without logging payloads', () => {
    expect(errorMessage(new Error('core failed'))).toBe('core failed');
    expect(errorMessage('bridge failed')).toBe('bridge failed');
    expect(errorMessage({ message: 'serialized failure', secret: 'do not show' })).toBe('serialized failure');
    expect(errorMessage({ code: 500 })).toBe('The reader core returned an unknown error.');
  });
});
