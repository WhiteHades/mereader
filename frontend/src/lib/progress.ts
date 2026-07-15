import type { ProgressPosition } from './types';

type SaveProgress = (position: ProgressPosition) => Promise<unknown>;

export class ProgressSaveQueue {
  private pending: ProgressPosition | null = null;
  private running: Promise<void> | null = null;
  private lastError: unknown = null;

  constructor(
    private readonly save: SaveProgress,
    private readonly onError: (error: unknown) => void,
  ) {}

  enqueue(position: ProgressPosition): void {
    this.pending = position;
    this.lastError = null;
    this.start();
  }

  async flush(): Promise<void> {
    await this.running;
    if (this.pending) {
      this.lastError = null;
      this.start();
      await this.running;
    }
    if (this.pending) throw this.lastError;
  }

  private start(): void {
    if (this.running) return;
    this.running = this.drain().finally(() => {
      this.running = null;
      if (this.pending && !this.lastError) this.start();
    });
  }

  private async drain(): Promise<void> {
    while (this.pending) {
      const position = this.pending;
      this.pending = null;
      try {
        await this.save(position);
      } catch (error) {
        this.pending ??= position;
        this.lastError = error;
        this.onError(error);
        return;
      }
    }
  }
}
