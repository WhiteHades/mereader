import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import AiPanel from '../components/AiPanel.svelte';
import type { AiEvent, AnswerResponse } from '../lib/types';

const mocks = vi.hoisted(() => ({
  getAiStatus: vi.fn(),
  askBook: vi.fn(),
  cancelAiRequest: vi.fn(),
  reindexBook: vi.fn(),
}));

vi.mock('../lib/commands', () => ({
  getAiStatus: mocks.getAiStatus,
  askBook: mocks.askBook,
  cancelAiRequest: mocks.cancelAiRequest,
  reindexBook: mocks.reindexBook,
  errorMessage: (error: unknown) => error instanceof Error ? error.message : String(error),
}));

const response: AnswerResponse = {
  answer: 'The argument depends on local evidence [S1].',
  progressBoundary: { currentLocation: 80, completionPercentage: 40 },
  sources: [
    {
      chapterId: 'chapter-2',
      chapterTitle: 'Second Chapter',
      citationId: 'S1',
      text: 'A grounded source passage.',
      startLocation: 72,
      endLocation: 78,
      relevanceScore: 0.92,
      retrievalMethods: ['keyword', 'vector'],
    },
  ],
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => { resolve = resolvePromise; });
  return { promise, resolve };
}

describe('AiPanel', () => {
  beforeEach(() => {
    mocks.getAiStatus.mockResolvedValue({ state: 'ready', embeddingModelAvailable: true });
    mocks.reindexBook.mockResolvedValue({ state: 'ready', embeddingModelAvailable: true });
    mocks.cancelAiRequest.mockResolvedValue(true);
  });

  it('shows streamed deltas, the boundary, and reopenable source disclosure', async () => {
    const returned = deferred<AnswerResponse>();
    const onJumpToSource = vi.fn();
    mocks.askBook.mockImplementation(
      async (
        _bookId: string,
        _requestId: string,
        _question: string,
        onEvent: (event: AiEvent) => void,
      ) => {
        onEvent({ type: 'status', message: 'Reading indexed passages' });
        onEvent({ type: 'delta', delta: 'Streaming words' });
        return returned.promise;
      },
    );
    render(AiPanel, { bookId: 'book-1', open: true, onClose: vi.fn(), onJumpToSource });

    const input = await screen.findByLabelText('Question about this book');
    await fireEvent.input(input, { target: { value: 'What is the argument?' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Ask' }));

    expect(await screen.findByText('Streaming words')).toBeTruthy();
    expect(screen.getByText('Reading indexed passages')).toBeTruthy();
    returned.resolve(response);

    expect(await screen.findByRole('button', { name: 'Jump to source S1' })).toBeTruthy();
    expect(screen.getByLabelText('AI answer').textContent).toContain(response.answer);
    expect(screen.getByText('Grounded through location 80 (40% of the book).')).toBeTruthy();
    const disclosure = screen.getByRole('button', { name: /Grounded sources/ });
    expect(screen.getByText('A grounded source passage.')).toBeTruthy();
    expect(screen.getByText('S1')).toBeTruthy();
    expect(screen.getByText('Rank 1 / Keyword + Vector')).toBeTruthy();
    expect(screen.queryByText('92% match')).toBeNull();

    await fireEvent.click(disclosure);
    expect(screen.queryByText('A grounded source passage.')).toBeNull();
    await fireEvent.click(disclosure);
    expect(screen.getByText('A grounded source passage.')).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Jump to source S1 at location 72' }));
    expect(onJumpToSource).toHaveBeenCalledWith(response.sources[0]);

    await fireEvent.click(screen.getByRole('button', { name: 'Clear answer' }));
    expect(screen.queryByLabelText('AI answer')).toBeNull();
    expect((input as HTMLTextAreaElement).value).toBe('');
  });

  it('uses the returned non-streaming answer and keeps it after retry failure', async () => {
    mocks.askBook.mockResolvedValueOnce(response).mockRejectedValueOnce(new Error('Ollama stopped'));
    render(AiPanel, { bookId: 'book-1', open: true, onClose: vi.fn(), onJumpToSource: vi.fn() });

    const input = await screen.findByLabelText('Question about this book');
    await fireEvent.input(input, { target: { value: 'First question' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Ask' }));
    expect(await screen.findByRole('button', { name: 'Jump to source S1' })).toBeTruthy();

    await fireEvent.input(input, { target: { value: 'Second question' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Ask' }));
    expect(await screen.findByText('Ollama stopped')).toBeTruthy();
    expect(screen.getByLabelText('AI answer').textContent).toContain(response.answer);
  });

  it.each([
    ['unavailable', 'Ollama is not available'],
    ['indexing', 'Indexing this book'],
    ['error', 'Semantic indexing needs attention'],
  ] as const)('renders the %s core state', async (state, label) => {
    mocks.getAiStatus.mockResolvedValue({ state, message: 'Status detail', indexedThroughLocation: 10 });
    render(AiPanel, { bookId: 'book-1', open: true, onClose: vi.fn(), onJumpToSource: vi.fn() });
    expect(await screen.findByText(label)).toBeTruthy();
    expect(mocks.getAiStatus).toHaveBeenCalledWith('book-1');
  });

  it('recovers from a status request error', async () => {
    mocks.getAiStatus
      .mockRejectedValueOnce(new Error('bridge offline'))
      .mockResolvedValueOnce({ state: 'ready' });
    render(AiPanel, { bookId: 'book-1', open: true, onClose: vi.fn(), onJumpToSource: vi.fn() });

    expect(await screen.findByText('bridge offline')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Retry status check' }));
    await waitFor(() => expect(screen.getByLabelText('Question about this book')).toBeTruthy());
  });

  it('keeps keyword questions usable and refreshes after reindexing', async () => {
    mocks.getAiStatus
      .mockResolvedValueOnce({
        state: 'ready',
        message: 'Keyword grounding remains ready.',
        embeddingModelAvailable: true,
        textOnlyBooks: 1,
      })
      .mockResolvedValueOnce({ state: 'ready', embeddingModelAvailable: true });
    mocks.reindexBook.mockResolvedValue({ state: 'indexing', embeddingModelAvailable: true });
    render(AiPanel, { bookId: 'book-1', open: true, onClose: vi.fn(), onJumpToSource: vi.fn() });

    expect(await screen.findByLabelText('Question about this book')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Reindex book' }));

    await waitFor(() => expect(mocks.reindexBook).toHaveBeenCalledWith('book-1'));
    await waitFor(() => expect(mocks.getAiStatus).toHaveBeenCalledTimes(2));
    expect(screen.queryByRole('button', { name: 'Reindex book' })).toBeNull();
  });

  it('turns only exact returned citation markers into buttons and leaves markup literal', async () => {
    mocks.askBook.mockResolvedValue({
      ...response,
      answer: 'Supported [S1], unknown [S2], padded [S01], and <em>literal</em>.',
    });
    render(AiPanel, { bookId: 'book-1', open: true, onClose: vi.fn(), onJumpToSource: vi.fn() });

    const input = await screen.findByLabelText('Question about this book');
    await fireEvent.input(input, { target: { value: 'Show citations' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Ask' }));

    expect(await screen.findByRole('button', { name: 'Jump to source S1' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Jump to source S2' })).toBeNull();
    const answerCopy = screen.getByLabelText('AI answer');
    expect(answerCopy.textContent).toContain('unknown [S2], padded [S01], and <em>literal</em>.');
    expect(answerCopy.querySelector('em')).toBeNull();
  });

  it('cancels the active native request when stopped', async () => {
    const returned = deferred<AnswerResponse>();
    let requestId = '';
    mocks.askBook.mockImplementation(
      async (_bookId: string, id: string) => {
        requestId = id;
        return returned.promise;
      },
    );
    render(AiPanel, { bookId: 'book-1', open: true, onClose: vi.fn(), onJumpToSource: vi.fn() });

    const input = await screen.findByLabelText('Question about this book');
    await fireEvent.input(input, { target: { value: 'Stop this request' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Ask' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Stop' }));

    expect(requestId).toMatch(/^[0-9a-f-]{36}$/);
    expect(mocks.cancelAiRequest).toHaveBeenCalledWith(requestId);
    returned.resolve(response);
  });

  it('cancels an active request when any parent close path hides the panel', async () => {
    const returned = deferred<AnswerResponse>();
    let requestId = '';
    mocks.askBook.mockImplementation(async (_bookId: string, id: string) => {
      requestId = id;
      return returned.promise;
    });
    const view = render(AiPanel, {
      bookId: 'book-1',
      open: true,
      onClose: vi.fn(),
      onJumpToSource: vi.fn(),
    });
    const input = await screen.findByLabelText('Question about this book');
    await fireEvent.input(input, { target: { value: 'Hide the panel' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Ask' }));

    await view.rerender({
      bookId: 'book-1',
      open: false,
      onClose: vi.fn(),
      onJumpToSource: vi.fn(),
    });

    await waitFor(() => expect(mocks.cancelAiRequest).toHaveBeenCalledWith(requestId));
    returned.resolve(response);
  });
});
