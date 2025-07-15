import { describe, it, expect, vi, beforeEach, afterEach, beforeAll } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createRouter, createWebHashHistory } from 'vue-router';
import { createPinia, setActivePinia } from 'pinia';
import App from '@/App.vue';
import LibraryView from '@/views/LibraryView.vue';
import ReaderView from '@/views/ReaderView.vue';
import { useThemeStore } from '@/services/themeStore';
import { apiService } from '@/services/apiService';

vi.mock('@tauri-apps/api/dialog', () => ({
  open: vi.fn().mockResolvedValue('/path/to/test.epub'),
}));

vi.mock('@tauri-apps/api/fs', () => ({
  readBinaryFile: vi.fn().mockResolvedValue(new Uint8Array([1, 2, 3])),
  readTextFile: vi.fn().mockResolvedValue('<html><body><p>Test content</p></body></html>'),
}));

vi.mock('@tauri-apps/api/path', () => ({
  basename: vi.fn().mockResolvedValue('test.epub'),
}));

vi.mock('@/services/apiService', () => ({
  apiService: {
    getBooks: vi.fn().mockResolvedValue({
      books: [
        {
          id: 'book1',
          title: 'Test Book 1',
          author: 'Test Author 1',
          cover_path: null,
          progress: 25
        },
        {
          id: 'book2',
          title: 'Test Book 2',
          author: 'Test Author 2',
          cover_path: null,
          progress: 50
        }
      ],
      total: 2
    }),
    uploadBook: vi.fn().mockResolvedValue({
      id: 'book3',
      title: 'New Book',
      author: 'New Author'
    }),
    getBookDetails: vi.fn().mockResolvedValue({
      id: 'book1',
      title: 'Test Book 1',
      author: 'Test Author 1',
      cover_path: null,
      total_locations: 100,
      chapters: [
        { id: 'ch1', title: 'Chapter 1', order: 1, start_location: 1, end_location: 50 },
        { id: 'ch2', title: 'Chapter 2', order: 2, start_location: 51, end_location: 100 }
      ]
    }),
    saveProgress: vi.fn().mockResolvedValue({
      current_location: 25,
      completion_percentage: 25.0
    })
  }
}));

let pinia;
let router;

function createTestRouter() {
  return createRouter({
    history: createWebHashHistory(),
    routes: [
      {
        path: '/',
        name: 'library',
        component: LibraryView
      },
      {
        path: '/reader/:id',
        name: 'reader',
        component: ReaderView,
        props: true
      }
    ]
  });
}

describe('MeReader Application Integration', () => {
  let wrapper;

  beforeAll(() => {
    Object.defineProperty(window, 'localStorage', {
      value: {
        getItem: vi.fn(),
        setItem: vi.fn(),
        removeItem: vi.fn(),
        clear: vi.fn(),
      },
      writable: true
    });

    pinia = createPinia();
    setActivePinia(pinia);

    window.HTMLMediaElement.prototype.load = vi.fn();
    window.HTMLMediaElement.prototype.play = vi.fn();
    window.HTMLMediaElement.prototype.pause = vi.fn();
  });

  beforeEach(async () => {
    vi.clearAllMocks();

    router = createTestRouter();

    wrapper = mount(App, {
      global: {
        plugins: [router, pinia],
        stubs: {
          ContentViewer: true,
          transition: true,
          RouterView: false
        },

        mocks: {
          fetch: vi.fn().mockImplementation((url) => {
            return Promise.resolve({
              ok: true,
              json: () => Promise.resolve({}),
              status: 200
            });
          })
        }
      }
    });

    const themeStore = useThemeStore();
    themeStore.setTheme('dark');

    await flushPromises();
  });

  afterEach(() => {
    wrapper.unmount();
  });

  it('should render the app with library view by default', async () => {
    await router.push('/');
    await flushPromises();

    expect(wrapper.classes()).toContain('app');

    expect(router.currentRoute.value.name).toBe('library');
  });

  it('should navigate from library to reader view', async () => {
    await router.push('/');
    await flushPromises();

    await router.push('/reader/book1');
    await flushPromises();

    expect(router.currentRoute.value.name).toBe('reader');
    expect(router.currentRoute.value.params.id).toBe('book1');
  });

  it('should apply theme changes across the application', async () => {
    const themeStore = useThemeStore();

    themeStore.setTheme('sepia');
    await flushPromises();

    expect(wrapper.classes()).toContain('app');
    expect(document.body.className).toBe('theme-sepia');

    themeStore.setTheme('light');
    await flushPromises();

    expect(document.body.className).toBe('');
  });
});

describe('Navigation Flow Integration', () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    router = createTestRouter();
    await router.push('/');
  });

  it('should handle book upload flow', async () => {
    const libraryViewWrapper = mount(LibraryView, {
      global: {
        plugins: [router, pinia],
        stubs: {
          PlusCircleIcon: true
        }
      }
    });

    const mockFile = new File(['epub content'], 'test.epub', { type: 'application/epub+zip' });
    const mockFileList = {
      0: mockFile,
      length: 1,
      item: (idx) => mockFile
    };

    const addButton = libraryViewWrapper.find('.btn');
    expect(addButton.exists()).toBe(true);

    libraryViewWrapper.vm.openBookDialog = vi.fn().mockImplementation(async () => {
      await libraryViewWrapper.vm.uploadFile('/path/to/test.epub');
      await libraryViewWrapper.vm.fetchBooks();
    });

    await addButton.trigger('click');
    await flushPromises();

    expect(apiService.uploadBook).toHaveBeenCalled();
    expect(apiService.getBooks).toHaveBeenCalled();
  });
});