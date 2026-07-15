import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import AiPanel from '../components/AiPanel.svelte';
import type { AiEvent, AnswerResponse } from '../lib/types';

const mocks = vi.hoisted(() => ({
  getAiStatus: vi.fn(),
  askBook: vi.fn(),
}));

vi.mock('../lib/commands', () => ({
  getAiStatus: mocks.getAiStatus,
  askBook: mocks.askBook,
  errorMessage: (error: unknown) => error instanceof Error ? error.message : String(error),
}));

const response: AnswerResponse = {
  answer: 'The argument depends on local evidence.',
  progressBoundary: { currentLocation: 80, completionPercentage: 40 },
  sources: [
    {
      chapterId: 'chapter-2',
      chapterTitle: 'Second Chapter',
      text: 'A grounded source passage.',
      startLocation: 72,
      endLocation: 78,
      relevanceScore: 0.92,
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
    mocks.getAiStatus.mockResolvedValue({ state: 'ready' });
  });

  it('shows streamed deltas, the boundary, and reopenable source disclosure', async () => {
    const returned = deferred<AnswerResponse>();
    const onJumpToSource = vi.fn();
    mocks.askBook.mockImplementation(
      async (_bookId: string, _question: string, onEvent: (event: AiEvent) => void) => {
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

    expect(await screen.findByText(response.answer)).toBeTruthy();
    expect(screen.getByText('Grounded through location 80 (40% of the book).')).toBeTruthy();
    const disclosure = screen.getByRole('button', { name: /Grounded sources/ });
    expect(screen.getByText('A grounded source passage.')).toBeTruthy();

    await fireEvent.click(disclosure);
    expect(screen.queryByText('A grounded source passage.')).toBeNull();
    await fireEvent.click(disclosure);
    expect(screen.getByText('A grounded source passage.')).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Jump to location 72' }));
    expect(onJumpToSource).toHaveBeenCalledWith(response.sources[0]);
  });

  it('uses the returned non-streaming answer and keeps it after retry failure', async () => {
    mocks.askBook.mockResolvedValueOnce(response).mockRejectedValueOnce(new Error('Ollama stopped'));
    render(AiPanel, { bookId: 'book-1', open: true, onClose: vi.fn(), onJumpToSource: vi.fn() });

    const input = await screen.findByLabelText('Question about this book');
    await fireEvent.input(input, { target: { value: 'First question' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Ask' }));
    expect(await screen.findByText(response.answer)).toBeTruthy();

    await fireEvent.input(input, { target: { value: 'Second question' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Ask' }));
    expect(await screen.findByText('Ollama stopped')).toBeTruthy();
    expect(screen.getByText(response.answer)).toBeTruthy();
  });

  it.each([
    ['unavailable', 'Ollama is not available'],
    ['indexing', 'Indexing this library'],
    ['error', 'The AI core reported an error'],
  ] as const)('renders the %s core state', async (state, label) => {
    mocks.getAiStatus.mockResolvedValue({ state, message: 'Status detail', indexedThroughLocation: 10 });
    render(AiPanel, { bookId: 'book-1', open: true, onClose: vi.fn(), onJumpToSource: vi.fn() });
    expect(await screen.findByText(label)).toBeTruthy();
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
});
