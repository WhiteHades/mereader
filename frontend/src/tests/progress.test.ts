import { describe, expect, it, vi } from 'vitest';
import { ProgressSaveQueue } from '../lib/progress';
import type { ProgressPosition } from '../lib/types';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

const position = (currentLocation: number): ProgressPosition => ({
  bookId: 'book-1',
  currentLocation,
  currentChapterId: 'chapter-1',
});

describe('reader progress save ordering', () => {
  it('serializes writes and saves only the latest pending position', async () => {
    const first = deferred<void>();
    const saved: number[] = [];
    const save = vi.fn(async (next: ProgressPosition) => {
      saved.push(next.currentLocation);
      if (next.currentLocation === 10) await first.promise;
    });
    const queue = new ProgressSaveQueue(save, vi.fn());

    queue.enqueue(position(10));
    queue.enqueue(position(20));
    queue.enqueue(position(30));

    expect(saved).toEqual([10]);
    first.resolve();
    await queue.flush();

    expect(saved).toEqual([10, 30]);
    expect(save).toHaveBeenCalledTimes(2);
  });

  it('keeps a failed latest position for a recoverable flush retry', async () => {
    const onError = vi.fn();
    const save = vi.fn()
      .mockRejectedValueOnce(new Error('disk busy'))
      .mockResolvedValueOnce(undefined);
    const queue = new ProgressSaveQueue(save, onError);

    queue.enqueue(position(55));
    await Promise.resolve();
    await expect(queue.flush()).resolves.toBeUndefined();

    expect(onError).toHaveBeenCalledWith(expect.objectContaining({ message: 'disk busy' }));
    expect(save).toHaveBeenCalledTimes(2);
  });
});
