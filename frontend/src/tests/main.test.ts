import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ mount: vi.fn() }));

vi.mock('svelte', async (importOriginal) => ({
  ...(await importOriginal<typeof import('svelte')>()),
  mount: mocks.mount,
}));
vi.mock('../App.svelte', () => ({ default: {} }));

describe('application entrypoint', () => {
  beforeEach(() => {
    vi.resetModules();
    document.body.innerHTML = '<div id="app"></div>';
  });

  it('mounts the Svelte app into the application root', async () => {
    await import('../main');
    expect(mocks.mount).toHaveBeenCalledWith(expect.anything(), {
      target: document.getElementById('app'),
    });
  });
});
