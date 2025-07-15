<template>
  <div class="content-viewer" :class="theme" ref="readerContainer">
    <!-- loading -->
    <div v-if="loading" class="loading-overlay">
      <div class="loading-spinner"></div>
      <p>Loading book...</p>
    </div>

    <!-- error -->
    <div v-if="error" class="error-overlay">
      <div class="error-icon">⚠️</div>
      <h3>{{ error }}</h3>
      <button @click="reload" class="retry-button">Retry</button>
    </div>

    <!-- reader container -->
    <div class="content-container" ref="contentContainer" v-show="!loading && !error">
      <div class="book-content" ref="bookContent" v-html="processedContent"></div>
    </div>
  </div>
</template>

<script>
import { ref, onMounted, onBeforeUnmount, watch, nextTick, computed } from 'vue';

export default {
  name: 'ContentViewer',
  props: {
    bookId: {
      type: String,
      required: true
    },
    initialLocation: {
      type: Number,
      default: 1
    },
    initialProgress: {
      type: Number,
      default: 0
    },
    apiBaseUrl: {
      type: String,
      default: 'http://localhost:8000/api'
    },
    theme: {
      type: String,
      default: 'dark'
    }
  },

  emits: ['locationChanged', 'ready', 'error', 'chapterChanged'],

  setup(props, { emit }) {
    const loading = ref(true);
    const error = ref(null);
    const contentContainer = ref(null);
    const readerContainer = ref(null);
    const bookContent = ref(null);

    const currentContent = ref('');
    const currentLocation = ref(props.initialLocation || 1);
    const currentChapter = ref(null);
    const chapters = ref([]);
    const totalLocations = ref(0);
    const isNavigating = ref(false);
    const scrollThrottle = ref(null);
    const debugMode = ref(true);
    const showChapterEndMessage = ref(false);
    const hasScrolledToInitialPosition = ref(false);
    const processedContent = computed(() => {
      return cleanHtmlContent(currentContent.value);
    });

    watch(() => props.theme, (newTheme) => {
      if (newTheme) {
        applyThemeToContent(newTheme);
      }
    });

    function handleResize() {
      if (contentContainer.value) {
        if (currentChapter.value && currentLocation.value) {
          scrollToLocation(currentLocation.value);
        }
      }
    }

    function applyThemeToContent(theme) {
      if (bookContent.value) {
        const element = bookContent.value;
        element.classList.remove('light', 'dark', 'sepia');
        element.classList.add(theme);
      }
    }

    async function initReader() {
      loading.value = true;
      error.value = null;
      hasScrolledToInitialPosition.value = false;

      try {
        await nextTick();

        if (!contentContainer.value) {
          throw new Error('Content container element not found');
        }

        const bookDetailsResponse = await fetch(`${props.apiBaseUrl}/books/${props.bookId}`);
        if (!bookDetailsResponse.ok) {
          throw new Error(`Failed to fetch book details: ${bookDetailsResponse.status}`);
        }

        const bookDetails = await bookDetailsResponse.json();
        totalLocations.value = bookDetails.total_locations || 0;
        chapters.value = bookDetails.chapters || [];

        if (debugMode.value) {
          console.log(`[Init] Total locations: ${totalLocations.value}, Chapters: ${chapters.value.length}`);
          console.log(`[Init] Initial location provided: ${props.initialLocation}`);
        }

        let targetLocation = props.initialLocation > 0 ? props.initialLocation : 1;
        let targetChapter = findChapterByLocation(targetLocation);

        if (!targetChapter && chapters.value.length > 0) {
          targetChapter = chapters.value[0];
          targetLocation = targetChapter.start_location;

          if (debugMode.value) {
            console.log(`[Init] No chapter found for location ${props.initialLocation}, using first chapter starting at ${targetLocation}`);
          }
        }

        if (targetChapter) {
          await loadChapterContent(targetChapter, targetLocation);
        } else {
          throw new Error('No chapters found in the book');
        }

        emit('ready', {
          totalLocations: totalLocations.value,
          chapters: chapters.value
        });

        loading.value = false;

      } catch (err) {
        console.error('Error initializing content reader:', err);
        error.value = `Failed to load book: ${err.message}`;
        loading.value = false;
        emit('error', err);
      }
    }

    async function loadChapterContent(chapter, targetLocation) {
      try {
        isNavigating.value = true;
        showChapterEndMessage.value = false;
        hasScrolledToInitialPosition.value = false;

        if (debugMode.value) {
          console.log(`[Chapter] Loading chapter ${chapter.title} with target location ${targetLocation}`);
        }

        const response = await fetch(`${props.apiBaseUrl}/content/chapter/${props.bookId}/${chapter.id}`);

        if (!response.ok) {
          throw new Error(`Failed to fetch chapter content: ${response.status}`);
        }

        const data = await response.json();

        currentContent.value = processImagePaths(data.content, props.bookId, props.apiBaseUrl);
        currentChapter.value = chapter;

        emit('chapterChanged', chapter);
        await nextTick();
        applyThemeToContent(props.theme);

        setTimeout(() => {
          scrollToLocation(targetLocation);
          hasScrolledToInitialPosition.value = true;
          setupScrollHandler();

          isNavigating.value = false;
        }, 100);

      } catch (err) {
        console.error('Error loading chapter content:', err);
        error.value = `Failed to load chapter content: ${err.message}`;
        isNavigating.value = false;
      }
    }

    function findChapterByLocation(location) {
      return chapters.value.find(chapter =>
        location >= chapter.start_location && location <= chapter.end_location
      );
    }

    function scrollToLocation(location) {
      if (!currentChapter.value || !contentContainer.value || !bookContent.value) return;

      const chapterLength = currentChapter.value.end_location - currentChapter.value.start_location + 1;
      const locationWithinChapter = location - currentChapter.value.start_location;

      const scrollPercentage = Math.min(1, Math.max(0, locationWithinChapter / chapterLength));

      const maxScroll = bookContent.value.scrollHeight - contentContainer.value.clientHeight;
      const scrollPosition = Math.max(0, Math.min(maxScroll, scrollPercentage * maxScroll));

      contentContainer.value.scrollTop = scrollPosition;

      if (debugMode.value) {
        console.log(`[Scroll] Scrolled to location ${location}, position ${Math.round(scrollPosition)}px (${Math.round(scrollPercentage * 100)}%)`);
        console.log(`[Scroll] Chapter range: ${currentChapter.value.start_location}-${currentChapter.value.end_location}, Content height: ${bookContent.value.scrollHeight}px, Max scroll: ${maxScroll}px`);
      }

      currentLocation.value = location;

      const progress = totalLocations.value > 0
        ? (location / totalLocations.value) * 100
        : 0;

      emit('locationChanged', {
        location: location,
        chapterId: currentChapter.value.id,
        chapterTitle: currentChapter.value.title,
        progress: progress
      });
    }

    function setupScrollHandler() {
      nextTick(() => {
        if (!contentContainer.value || !bookContent.value) return;
        contentContainer.value.removeEventListener('scroll', handleScroll);
        contentContainer.value.addEventListener('scroll', handleScroll);
        if (debugMode.value) {
          console.log('[Scroll] Scroll handler set up');
        }
      });
    }

    function handleScroll(event) {
      if (!hasScrolledToInitialPosition.value) return;
      if (scrollThrottle.value) return;

      scrollThrottle.value = setTimeout(() => {
        scrollThrottle.value = null;
        if (!contentContainer.value || !bookContent.value || !currentChapter.value) return;

        const scrollTop = contentContainer.value.scrollTop;
        const scrollHeight = bookContent.value.scrollHeight;
        const containerHeight = contentContainer.value.clientHeight;
        const maxScroll = scrollHeight - containerHeight;

        const scrollPercentage = maxScroll <= 0 ? 0 : Math.min(1, Math.max(0, scrollTop / maxScroll));

        const chapterLength = currentChapter.value.end_location - currentChapter.value.start_location + 1;
        const newLocation = Math.floor(currentChapter.value.start_location + (chapterLength * scrollPercentage));

        const boundedLocation = Math.max(
          currentChapter.value.start_location,
          Math.min(newLocation, currentChapter.value.end_location)
        );

        const isNearEnd = scrollTop + containerHeight >= scrollHeight - 20;

        if (isNearEnd && !showChapterEndMessage.value) {
          showChapterEndMessage.value = true;

          if (debugMode.value) {
            console.log('[Scroll] Reached end of chapter');
          }
        } else if (!isNearEnd && showChapterEndMessage.value) {
          showChapterEndMessage.value = false;
        }

        if (boundedLocation !== currentLocation.value) {
          currentLocation.value = boundedLocation;

          const progress = totalLocations.value > 0
            ? (boundedLocation / totalLocations.value) * 100
            : 0;

          emit('locationChanged', {
            location: boundedLocation,
            chapterId: currentChapter.value.id,
            chapterTitle: currentChapter.value.title,
            progress: progress
          });

          if (debugMode.value) {
            console.log(`[Scroll] Updated location to ${boundedLocation} (${Math.round(progress)}%)`);
          }
        }
      }, 150);
    }

    function nextChapter() {
      if (isNavigating.value) return;

      const currentChapterIndex = chapters.value.findIndex(ch => ch.id === currentChapter.value?.id);
      if (currentChapterIndex < chapters.value.length - 1) {
        const nextChapter = chapters.value[currentChapterIndex + 1];
        loadChapterContent(nextChapter, nextChapter.start_location);
      }
    }

    function prevChapter() {
      if (isNavigating.value) return;

      const currentChapterIndex = chapters.value.findIndex(ch => ch.id === currentChapter.value?.id);
      if (currentChapterIndex > 0) {
        const prevChapter = chapters.value[currentChapterIndex - 1];
        loadChapterContent(prevChapter, prevChapter.start_location);
      }
    }

    async function goToLocation(location) {
      if (isNavigating.value) return;
      const targetLocation = Math.max(1, Math.min(location, totalLocations.value || 1));
      const targetChapter = findChapterByLocation(targetLocation);

      if (!targetChapter) {
        console.error(`No chapter found for location: ${targetLocation}`);
        return;
      }

      if (currentChapter.value && targetChapter.id === currentChapter.value.id) {
        scrollToLocation(targetLocation);
      } else {
        await loadChapterContent(targetChapter, targetLocation);
      }
    }

    function handleKeyPress(e) {
      if (loading.value || error.value) return;

      if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') return;

      switch (e.key) {
        case 'ArrowRight':
          e.preventDefault();
          nextChapter();
          break;
        case 'ArrowLeft':
          e.preventDefault();
          prevChapter();
          break;
        case ' ':
          e.preventDefault();
          if (contentContainer.value) {
            const isAtEnd = contentContainer.value.scrollTop + contentContainer.value.clientHeight >= bookContent.value.scrollHeight - 20;
            if (isAtEnd) {
              nextChapter();
            } else {
              contentContainer.value.scrollBy({ top: contentContainer.value.clientHeight * 0.9, behavior: 'smooth' });
            }
          }
          break;
      }
    }

    function reload() {
      initReader();
    }

    function processImagePaths(html, bookId, apiBaseUrl) {
      try {
        const parser = new DOMParser();
        const doc = parser.parseFromString(html, 'text/html');

        // Debug: Log all images found
        console.log("Processing images in chapter");

        // Update image paths
        const images = doc.querySelectorAll('img');
        console.log(`Found ${images.length} images in chapter`);

        images.forEach((img, index) => {
          const srcAttr = img.getAttribute('src');
          console.log(`Image ${index} original src: ${srcAttr}`);

          if (srcAttr && !srcAttr.startsWith('http') && !srcAttr.startsWith('data:')) {
            // Get just the filename, strip any paths
            const filename = srcAttr.split('/').pop();
            console.log(`Extracted filename: ${filename}`);

            // Set the full URL to the image endpoint
            const newSrc = `${apiBaseUrl}/content/image/${bookId}/${filename}`;
            img.setAttribute('src', newSrc);
            console.log(`Updated src to: ${newSrc}`);
          }
        });

        return new XMLSerializer().serializeToString(doc);
      } catch (error) {
        console.error("Error processing image paths:", error);
        return html;
      }
    }

    function cleanHtmlContent(html) {
      if (!html) return '';

      html = html
          .replace(/<\?xml[^>]+\?>/g, '')
          .replace(/<\/?(html|body)[^>]*>/gi, '')
          .replace(/^\s*html[\s.,:!?]*(?=<|$)/i, '')
          .replace(/^\s*html\b[\s.,:!?]*/i, '')
          .replace(/\s{2,}/g, ' ')
          .replace(/(\n\s*){2,}/g, '\n\n')
          .trim()

      const doc = new DOMParser().parseFromString(html, 'text/html');
      const firstNode = doc.body?.firstChild;

      if (firstNode?.textContent) {
        firstNode.textContent = firstNode.textContent.replace(/^\s*html[\s.,:!?]*/i, '');
      }

      html = new XMLSerializer().serializeToString(doc.body)
          .replace(/^<body[^>]*>/i, '')
          .replace(/<\/body>$/i, '')
          .trim();

      console.log('[CLEANED CONTENT]', html.slice(0, 200));
      return html;
    }

    onMounted(() => {
      document.addEventListener('keydown', handleKeyPress);
      window.addEventListener('resize', handleResize);
      initReader();
    });

    onBeforeUnmount(() => {
      document.removeEventListener('keydown', handleKeyPress);
      window.removeEventListener('resize', handleResize);
      if (scrollThrottle.value) { clearTimeout(scrollThrottle.value); }
    });

    return {
      loading,
      error,
      contentContainer,
      readerContainer,
      bookContent,
      currentContent,
      currentLocation,
      currentChapter,
      showChapterEndMessage,
      processedContent,
      nextChapter,
      prevChapter,
      nextPage: nextChapter,
      prevPage: prevChapter,
      goToLocation,
      reload,
    };
  }
};
</script>

<style scoped>
.content-viewer {
  position: relative;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background-color: var(--color-background);
  color: var(--color-text);
}

.content-container {
  width: 100%;
  height: calc(100% - 48px);
  overflow-y: auto;
  overflow-x: hidden;
  padding: 2rem 2rem 4rem 2rem;
  display: flex;
  flex-direction: column;
  scroll-behavior: smooth;
}

.book-content {
  max-width: 800px;
  min-height: 300px;
  margin: 0 auto;
  line-height: 1.6;
  font-size: 1.1rem;
  padding: 0 0 4rem 0;
  width: 100%;
  white-space: normal;
}

.loading-overlay,
.error-overlay {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  background-color: var(--color-background);
  z-index: 10;
  padding: 2rem;
  text-align: center;
  color: var(--color-text);
}

.loading-spinner {
  width: 40px;
  height: 40px;
  border: 3px solid var(--color-background-alt);
  border-top-color: var(--color-primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
  margin-bottom: 1rem;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.error-icon {
  font-size: 2rem;
  margin-bottom: 1rem;
  color: var(--color-danger);
}

.retry-button {
  margin-top: 1rem;
  padding: 0.5rem 1.5rem;
  background-color: var(--color-primary);
  color: white;
  border: none;
  border-radius: 8px;
  cursor: pointer;
}

.book-content {
  font-family: var(--reader-font), serif;
}

.chapter-end-message p {
  font-size: 1.1rem;
  margin-bottom: 1rem;
  color: var(--color-text);
}

@keyframes fadeIn {
  from { opacity: 0; transform: translateY(10px); }
  to { opacity: 1; transform: translateY(0); }
}

:deep(.dark) {
  color: #f5f5f7 !important;
  background-color: #1a1a1a !important;
}

:deep(.light) {
  color: #333 !important;
  background-color: #fff !important;
}

:deep(.sepia) {
  color: #5f4b32 !important;
  background-color: #f7f1e3 !important;
}

:deep(img) {
  max-width: 100%;
  height: auto;
  margin: 1rem 0;
}

:deep(a) {
  color: var(--color-primary);
  text-decoration: none;
}

:deep(h1), :deep(h2), :deep(h3), :deep(h4), :deep(h5), :deep(h6) {
  margin-top: 1.5em;
  margin-bottom: 0.8em;
  white-space: normal;
  overflow-wrap: break-word;
  word-break: break-word;
}

:deep(pre), :deep(code) {
  white-space: pre-wrap;
}

:deep(p) {
  margin-bottom: 1em;
}

:deep(img) {
  max-width: 100%;
  height: auto;
  display: block;
  margin: 1rem auto;
  object-fit: contain;
}
</style>
