<template>
  <div class="reader-view" :class="theme">
    <!-- content viewer -->
    <ContentViewer
      v-if="bookDetails"
      :bookId="bookId"
      :initialLocation="initialLocation"
      :initialProgress="readingProgress"
      :apiBaseUrl="apiBaseUrl"
      :theme="theme"
      @locationChanged="onLocationChanged"
      @ready="onBookReady"
      @error="onReaderError"
      @chapterChanged="onChapterChanged"
      class="content-view-container"
      ref="contentViewer"
    />

    <!-- TOC -->
    <transition name="slide-panel">
      <div v-if="showToc" class="toc-panel">
        <div class="panel-header">
          <h3>Contents</h3>
          <button class="close-button" @click="showToc = false"
                  aria-label="Close">
            <Bars3Icon class="icon" />
          </button>
        </div>
        <ul class="toc-list">
          <li
              v-for="chapter in chapters"
              :key="chapter.id"
              :class="{ active: currentChapter && currentChapter.id === chapter.id }"
              @click="navigateToChapter(chapter)"
          >
            {{ chapter.title }}
          </li>
        </ul>
      </div>
    </transition>

    <!-- AI -->
    <transition name="slide-panel">
      <div v-if="showAiPanel" class="ai-panel">
        <div class="panel-header">
          <h3>Ask about this book</h3>
          <button class="close-button" @click="showAiPanel = false"
                  aria-label="Close">
            <XMarkIcon class="icon" />
          </button>
        </div>

        <div class="query-form">
          <div class="query-input-container">
            <textarea
                v-model="queryText"
                placeholder="Ask a question about the book..."
                @keydown.enter.ctrl="submitQuery"
            ></textarea>

            <div class="query-buttons-row">
              <button
                  class="icon-button"
                  @click="submitQuery"
                  :disabled="isQueryLoading"
                  aria-label="Submit"
              >
                <span v-if="!isQueryLoading">
                  <PaperAirplaneIcon class="icon" />
                </span>
                <span v-show="isQueryLoading" class="mini-spinner"></span>
              </button>

              <button
                  class="icon-button"
                  @click.stop="clearQuery"
                  aria-label="Clear"
              >
                <XMarkIcon class="icon"/>
              </button>
            </div>
          </div>
        </div>

        <div v-if="queryResponse" class="query-response">
            <div class="response-content">
              {{ queryResponse.response }}
            </div>

            <div class="context-passages"
                 v-show="showContextPassages && queryResponse?.context_used?.length">
              <div class="context-header">
                <h4>Relevant Passages</h4>
                <button
                    class="toggle-button"
                    @click="showContextPassages = !showContextPassages"
                    aria-label="Toggle passages"
                >
                  <ChevronDownIcon class="icon" />
                </button>
              </div>
              <div
                  v-for="(passage, idx) in queryResponse.context_used"
                  :key="idx"
                  class="context-passage"
              >
                <div class="passage-header">
                  <span class="chapter-name">{{ passage.chapter_title }}</span>
                  <span class="relevance-badge"
                        :style="getRelevanceColor(passage.relevance_score)">
                  {{ Math.round(passage.relevance_score * 100) }}%
                </span>
                </div>
                <div class="passage-text">{{ cleanPassageText(passage.text) }}</div>
              </div>
            </div>
          </div>

      </div>
    </transition>

    <!-- floating title -->
    <div class="floating-book-title-area">
      <div class="floating-book-title">
        <button class="back-button" @click="goToLibrary"
                aria-label="Back to Library">
          <ArrowLeftIcon class="icon" height: />
        </button>
        <h2>{{ bookDetails ? bookDetails.title : 'Reading' }}</h2>
      </div>
    </div>

    <!-- floating controls -->
    <div class="floating-controls-area">
      <div class="floating-controls">
        <button class="control-button" @click="showToc = !showToc"
                aria-label="Contents">
          <Bars3Icon class="icon" />
        </button>
        <button class="control-button" @click="prevPage"
                aria-label="Previous Page">
          <ChevronLeftIcon class="icon" />
        </button>
        <div class="progress-display">{{ Math.round(readingProgress) }}%</div>
        <button class="control-button" @click="nextPage" aria-label="Next Page">
          <ChevronRightIcon class="icon" />
        </button>
        <button class="control-button" @click="showAiPanel = !showAiPanel"
                aria-label="Ask AI">
          <SparklesIcon class="icon"/>
        </button>
        <div class="theme-selector">
          <button
              class="theme-button"
              :class="{ active: selectedTheme  === 'light' }"
              @click="setTheme('light')"
              aria-label="Light theme"
          >
            <SunIcon class="icon"/>
          </button>
          <button
              class="theme-button"
              :class="{ active: selectedTheme  === 'sepia' }"
              @click="setTheme('sepia')"
              aria-label="Sepia theme"
          >
            <RectangleStackIcon class="icon"/>
          </button>
          <button
              class="theme-button"
              :class="{ active: selectedTheme  === 'dark' }"
              @click="setTheme('dark')"
              aria-label="Dark theme"
          >
            <MoonIcon class="icon"/>
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<script>
import { ArrowLeftIcon,ChevronDownIcon, Bars3Icon, ChevronLeftIcon,
  ChevronRightIcon, SparklesIcon, SunIcon, MoonIcon, RectangleStackIcon,
  PaperAirplaneIcon, XMarkIcon } from '@heroicons/vue/24/outline'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import ContentViewer from '@/components/ContentViewer.vue';
import { useThemeStore } from "@/services/themeStore.js";

export default {
  name: 'ReaderView',
  components: {
    ContentViewer,
    ArrowLeftIcon,
    Bars3Icon,
    ChevronLeftIcon,
    ChevronRightIcon,
    SparklesIcon,
    SunIcon,
    MoonIcon,
    RectangleStackIcon,
    PaperAirplaneIcon,
    XMarkIcon,
    ChevronDownIcon
  },

  setup() {
    const route = useRoute();
    const router = useRouter();

    const bookId = ref(route.params.id);
    const bookDetails = ref(null);
    const initialLocation = ref(1);
    const isLoading = ref(true);
    const error = ref(null);
    const apiBaseUrl = ref('http://localhost:8000/api');

    const contentViewer = ref(null);
    const chapters = ref([]);
    const currentChapter = ref(null);
    const readingProgress = ref(0);
    const currentLocation = ref(null);
    const totalLocations = ref(0);

    const showToc = ref(false);
    const showAiPanel = ref(false);
    const showContextPassages = ref(true);

    const queryText = ref('');
    const queryResponse = ref(null);
    const isQueryLoading = ref(false);

    const themeStore = useThemeStore();
    const theme = computed(() => themeStore.theme);
    const selectedTheme = ref(theme.value)

    const lastSavedLocation = ref(0);
    const isSaving = ref(false);

    const saveProgressTimeout = ref(null);

    watch(theme, (newTheme) => {
      document.body.className = newTheme === 'dark' ? 'theme-dark' :
          newTheme === 'sepia' ? 'theme-sepia' : '';
    });

    watch(() => route.params.id, (newId) => {
      if (newId && newId !== bookId.value) {
        bookId.value = newId;
        loadBookData();
      }
    });

    function goToLibrary() {
      saveProgressBeforeLeaving().then(() => {
        router.push('/');
      });
    }

    function setTheme(newTheme) {
      selectedTheme.value = newTheme;
      themeStore.setTheme(newTheme);
    }

    function clearQuery() {
      queryText.value = '';
      queryResponse.value = null;
    }

    function nextPage() {
      if (contentViewer.value && contentViewer.value.nextPage) {
        contentViewer.value.nextPage();
      }
    }

    function prevPage() {
      if (contentViewer.value && contentViewer.value.prevPage) {
        contentViewer.value.prevPage();
      }
    }

    function navigateToChapter(chapter) {
      if (contentViewer.value && contentViewer.value.goToLocation) {
        contentViewer.value.goToLocation(chapter.start_location);
        showToc.value = false;
      }
    }

    async function loadBookData() {
      try {
        isLoading.value = true;
        error.value = null;

        const response = await fetch(`${apiBaseUrl.value}/books/${bookId.value}`);
        if (!response.ok) {
          throw new Error(`Failed to fetch book details: ${response.status}`);
        }

        bookDetails.value = await response.json();

        chapters.value = bookDetails.value.chapters || [];
        totalLocations.value = bookDetails.value.total_locations || 0;

        await loadReadingProgress();

        isLoading.value = false;
      } catch (err) {
        console.error('Error loading book data:', err);
        error.value = `Failed to load book: ${err.message}`;
        isLoading.value = false;
      }
    }

    async function loadReadingProgress() {
      try {
        const response = await fetch(`${apiBaseUrl.value}/progress/${bookId.value}`);

        if (response.status === 404) {
          console.log(`No reading progress found for book: ${bookId.value}`);
          return null;
        }

        if (!response.ok) {
          throw new Error(`Failed to fetch reading progress: ${response.status}`);
        }

        const progress = await response.json();

        if (progress) {
          readingProgress.value = progress.completion_percentage || 0;
          initialLocation.value = progress.current_location || 1;
          lastSavedLocation.value = initialLocation.value;

          if (progress.current_chapter) {
            currentChapter.value = progress.current_chapter;
          }
        }
      } catch (err) {
        console.warn('Could not load reading progress:', err);
        initialLocation.value = 1;
        readingProgress.value = 0;
      }
    }

    function onBookReady(data) {
      if (data.chapters && data.chapters.length > 0) {
        chapters.value = data.chapters;
      }

      if (data.totalLocations) {
        totalLocations.value = data.totalLocations;
      }
    }

    function onChapterChanged(chapter) {
      currentChapter.value = chapter;
    }

    function onLocationChanged(location) {
      currentLocation.value = location;

      if (location.progress !== undefined) {
        readingProgress.value = location.progress;
      }

      saveProgress(location.location);
    }

    function onReaderError(err) {
      console.error('Reader error:', err);
      error.value = err.message;
    }

    function saveProgress(location) {
      if (saveProgressTimeout.value) {
        clearTimeout(saveProgressTimeout.value);
      }

      saveProgressTimeout.value = setTimeout(() => {
        saveReadingProgress(location, readingProgress.value);
      }, 2000);
    }

    async function saveReadingProgress(location, progressPercentage) {
      if (!bookId.value || !location || isSaving.value) return;

      try {
        isSaving.value = true;

        console.log(`Saving progress - location: ${location}, chapter: ${currentChapter.value?.id}, percentage: ${progressPercentage}`);

        const progressData = {
          current_location: parseInt(location),
          completion_percentage: progressPercentage
        };

        if (currentChapter.value && currentChapter.value.id) {
          progressData.chapter_id = currentChapter.value.id;
        }

        const response = await fetch(`${apiBaseUrl.value}/progress/${bookId.value}`, {
          method: 'PUT',
          body: JSON.stringify(progressData),
          headers: {
            'Content-Type': 'application/json'
          }
        });

        if (!response.ok) {
          let errorDetail = "";
          try {
            const errorData = await response.json();
            errorDetail = errorData.detail || "";
          } catch (parseErr) {
            errorDetail = response.statusText;
          }

          throw new Error(`HTTP error ${response.status}: ${errorDetail}`);
        }

        const result = await response.json();
        lastSavedLocation.value = location;
        console.log("Progress saved successfully", result);

        return result;
      } catch (err) {
        console.warn('Failed to save reading progress:', err);
      } finally {
        isSaving.value = false;
      }
    }

    async function saveProgressBeforeLeaving() {
      if (currentLocation.value && currentLocation.value.location) {
        try {
          const currentPosition = {
            current_location: currentLocation.value.location,
            chapter_id: currentChapter.value?.id,
            completion_percentage: readingProgress.value
          };

          const response = await fetch(`${apiBaseUrl.value}/progress/${bookId.value}`, {
            method: 'PUT',
            body: JSON.stringify(currentPosition),
            headers: {
              'Content-Type': 'application/json'
            }
          });

          if (!response.ok) {
            throw new Error(`HTTP error ${response.status}`);
          }

          return true;
        } catch (err) {
          console.error("Error saving final progress:", err);
          return false;
        }
      }
      return false;
    }

    async function submitQuery() {
      if (!queryText.value.trim() || isQueryLoading.value) return;

      isQueryLoading.value = true;
      try {
        const query = { query: queryText.value.trim() };

        const response = await fetch(`${apiBaseUrl.value}/query/ask/${bookId.value}`, {
          method: 'POST',
          body: JSON.stringify(query),
          headers: {
            'Content-Type': 'application/json'
          }
        });

        if (!response.ok) {
          const errorData = await response.json().catch(() => null);
          if (errorData && errorData.detail) { throw new Error(errorData.detail); }
          throw new Error(`Failed to process AI query: ${response.status}`);
        }

        queryResponse.value = await response.json();
      } catch (err) {
        console.error('Error submitting query:', err);
        alert(`Query error: ${err.message}`);
      } finally { isQueryLoading.value = false; }
    }

    function getRelevanceColor(score) {
      if (score > 0.8) return {backgroundColor: 'var(--color-secondary)'};
      if (score > 0.5) return {backgroundColor: 'var(--color-primary)'};
      return {backgroundColor: 'var(--color-text-secondary)'};
    }

    onMounted(() => {
      document.addEventListener('click', handleClickOutside, true);
      if (bookId.value) loadBookData();
      else router.push('/');
    });

    onBeforeUnmount(() => {
      document.removeEventListener('click', handleClickOutside, true);
      saveProgressBeforeLeaving();

      if (saveProgressTimeout.value) {
        clearTimeout(saveProgressTimeout.value);
      }
    });

    function handleClickOutside(event) {
      const tocEl = document.querySelector('.toc-panel');
      const aiEl = document.querySelector('.ai-panel');
      const tocBtn = document.querySelector('[aria-label="Contents"]');
      const aiBtn = document.querySelector('[aria-label="Ask AI"]');

      if (tocBtn?.contains(event.target) || aiBtn?.contains(event.target)) {
        return;
      }
      if (showToc.value && tocEl && !tocEl.contains(event.target)) showToc.value = false;
      if (showAiPanel.value && aiEl && !aiEl.contains(event.target)) showAiPanel.value = false;
    }

    function cleanPassageText(text) {
      return text.replace(/^(<\?xml[^>]+\?>|<!DOCTYPE[^>]+>|<html>)/g, '').trim();
    }

    return {
      bookId,
      bookDetails,
      initialLocation,
      isLoading,
      error,
      apiBaseUrl,

      chapters,
      currentChapter,
      readingProgress,
      contentViewer,
      totalLocations,

      theme,
      setTheme,
      selectedTheme,

      showToc,
      showAiPanel,
      showContextPassages,

      queryText,
      clearQuery,
      queryResponse,
      isQueryLoading,


      navigateToChapter,
      submitQuery,
      onBookReady,
      onChapterChanged,
      onLocationChanged,
      onReaderError,
      getRelevanceColor,
      saveProgressBeforeLeaving,
      nextPage,
      prevPage,
      goToLibrary,
      cleanPassageText
    };
  }
};
</script>

<style scoped>
.reader-view {
  height: 100%;
  display: flex;
  flex-direction: column;
  position: relative;
  background-color: var(--color-background);
  transition: padding-right 0.4s cubic-bezier(0.16, 1, 0.3, 1);
}

.content-view-container {
  flex: 1;
  position: relative;
  width: 100%;
  height: 100%;
  padding-top: 48px;
  padding-bottom: 48px;
}

.toc-panel {
  position: fixed;
  top: 16px;
  left: 16px;
  bottom: 16px;
  width: 280px;
  background-color: rgba(var(--color-surface-rgb), 0.85);
  backdrop-filter: blur(6px);
  -webkit-backdrop-filter: blur(6px);
  box-shadow: 2px 0 24px rgba(0, 0, 0, 0.15);
  z-index: 20;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border-radius: 8px;
  border: 1px solid var(--color-border);
}

.panel-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 12px 16px;
  border-bottom: 1px solid var(--color-border);
}

.panel-header h3 {
  font-size: 16px;
  font-weight: 600;
  margin: 0;
}

.close-button {
  background: none;
  border: none;
  color: var(--color-text-secondary);
  cursor: pointer;
  padding: 4px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background-color 0.2s;
}

.close-button:hover {
  background-color: var(--color-background-alt);
}

.toc-list {
  flex: 1;
  list-style: none;
  padding: 8px 0;
  margin: 0;
  overflow-y: auto;
}

.toc-list li {
  padding: 10px 16px;
  cursor: pointer;
  font-size: 14px;
  transition: background-color 0.2s;
  border-radius: 4px;
  margin: 0 8px;
}

.toc-list li:hover {
  background-color: var(--color-background-alt);
  color: var(--color-primary);
}

.toc-list li.active {
  background-color: var(--color-primary);
  color: white;
  font-weight: 500;
}

.ai-panel {
  position: fixed;
  top: 16px;
  right: 16px;
  bottom: 16px;
  width: 320px;
  box-shadow: -2px 0 24px rgba(0, 0, 0, 0.15);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border-radius: 8px;
  background-color: rgba(var(--color-surface-rgb), 0.85);
  backdrop-filter: blur(6px);
  -webkit-backdrop-filter: blur(6px);
  z-index: 50;
  border: 1px solid var(--color-border);
}

.query-form {
  padding: 16px;
  display: flex;
  flex-direction: column;
}

.query-input-container {
  position: relative;
  display: flex;
  flex-direction: column;
}

.query-buttons-row {
  margin-top: 8px;
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.icon-button {
  width: 40px;
  height: 28px;
  background-color: rgba(var(--color-surface-rgb), 0.75);
  color: var(--color-text);
  border: none;
  border-radius: 4px;
  cursor: pointer;
  display: flex;
  align-items: center;
  align-self: flex-end;
  justify-content: center;
  transition: all 0.2s ease;
  padding: 0;
}

.icon-button:hover {
  background-color: var(--color-primary);
  transform: scale(1.05);
}

.icon-button:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.query-input-container textarea {
  flex: 1;
  display: flex;
  flex-direction: column;
  width: 100%;
  height: auto;
  min-height: 120px;
  max-height: 240px;
  padding: 12px;
  font-size: 14px;
  border-radius: 6px;
  border: 1px solid var(--color-border);
  background-color: rgba(var(--color-background-rgb), 0.5);
  color: var(--color-text);
  resize: none;
  font-family: inherit;
  line-height: 1.4;
  overflow-y: auto;
  scrollbar-gutter: stable both-edges;
  scrollbar-width: thin;
}

.query-input-container textarea:focus {
  outline: none;
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px rgba(var(--color-primary-rgb), 0.2);
}

.mini-spinner {
  width: 16px;
  height: 16px;
  border: 2px solid rgba(255, 255, 255, 0.3);
  border-top-color: white;
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

.query-response {
  flex: 1;
  padding: 16px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.response-content {
  padding: 16px;
  background-color: rgba(var(--color-background-rgb), 0.5);
  border-radius: 6px;
  font-size: 14px;
  line-height: 1.6;
  white-space: pre-line;
  border: 1px solid var(--color-border);
}

.context-passages {
  margin-top: 12px;
}

.context-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 12px;
}

.context-header h4 {
  font-size: 14px;
  font-weight: 500;
  margin: 0;
  color: var(--color-text-secondary);
}

.toggle-button {
  background: none;
  border: none;
  color: var(--color-text-secondary);
  cursor: pointer;
  padding: 4px;
  border-radius: 4px;
}

.context-passage {
  margin-bottom: 12px;
  padding: 12px;
  background-color: rgba(var(--color-background-rgb), 0.5);
  border: 1px solid var(--color-border);
  border-radius: 6px;
  font-size: 13px;
}

.passage-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 8px;
}

.chapter-name {
  font-weight: 500;
  font-size: 12px;
}

.relevance-badge {
  font-size: 10px;
  padding: 2px 6px;
  border-radius: 8px;
  color: white;
  font-weight: 500;
}

.passage-text {
  font-size: 12px;
  line-height: 1.5;
  color: var(--color-text-secondary);
  max-height: 200px;
  overflow-y: auto;
  scrollbar-width: thin;
}

.back-button {
  width: 32px;
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  background-color: transparent;
  color: var(--color-text);
  border-radius: 4px;
  margin-right: 12px;
  cursor: pointer;
  transition: background-color 0.2s;
}

.icon {
  width: 20px;
  height: 20px;
  stroke-width: 2;
  color: var(--color-text);
}

.control-button {
  width: 38px;
  height: 38px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  background-color: transparent;
  color: var(--color-text);
  border-radius: 4px;
  cursor: pointer;
  transition: background-color 0.2s;
}

.control-button:hover {
  background-color: rgba(var(--color-background-rgb), 0.5);
}

.progress-display {
  font-size: 14px;
  font-weight: 500;
  min-width: 45px;
  text-align: center;
}

.theme-selector {
  display: flex;
  align-items: center;
  padding-left: 8px;
  margin-left: 8px;
  border-left: 1px solid var(--color-border);
  gap: 6px;
}

.theme-button {
  width: 28px;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
  border-radius: 4px;
  transition: none !important;
}

body {
  transition: background-color 0.4s ease, color 0.4s ease;
}

.query-input-container textarea,
.response-content,
.context-passage {
  font-family: var(--reader-font), serif;
}


.theme-button:hover {
  background-color: rgba(var(--color-background-rgb), 0.5);
}

.theme-button.active {
  color: var(--color-primary);
  background-color: rgba(var(--color-background-rgb), 0.5);
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.toc-list::-webkit-scrollbar,
.query-response::-webkit-scrollbar,
.query-input-container textarea::-webkit-scrollbar,
.passage-text::-webkit-scrollbar {
  width: 4px;
}

.toc-list::-webkit-scrollbar-thumb,
.query-response::-webkit-scrollbar-thumb,
.query-input-container textarea::-webkit-scrollbar-thumb,
.passage-text::-webkit-scrollbar-thumb {
  background-color: rgba(var(--color-text-rgb), 0.2);
  border-radius: 4px;
}

.toc-list::-webkit-scrollbar-track,
.query-response::-webkit-scrollbar-track,
.query-input-container textarea::-webkit-scrollbar-track,
.passage-text::-webkit-scrollbar-track {
  background: transparent;
}

.floating-book-title-area {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  height: 48px;
  z-index: 90;
  pointer-events: auto;
  padding: 4px 12px;
}

.floating-book-title-area:hover .floating-book-title {
  opacity: 1;
  visibility: visible;
}

.floating-controls-area:hover .floating-controls {
  opacity: 1;
  visibility: visible;
}

.floating-book-title {
  position: fixed;
  top: 16px;
  left: 50%;
  padding: 12px 24px;
  transform: translateX(-50%);
  background-color: rgba(var(--color-surface-rgb), 0.75);
  backdrop-filter: blur(10px);
  -webkit-backdrop-filter: blur(10px);
  border-radius: 8px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
  z-index: 100;
  display: flex;
  align-items: center;
  gap: 12px;
  opacity: 0;
  pointer-events: auto;
  border: 1px solid var(--color-border);
   visibility: hidden;
   transition: opacity 0.3s ease, visibility 0.3s ease;
 }

 .floating-book-title h2 {
   margin: 0;
   font-size: 15px;
   font-weight: 500;
   white-space: nowrap;
   overflow: hidden;
   text-overflow: ellipsis;
 }

 .back-button {
   width: 16px;
   height: 16px;
   background: none;
   border: none;
   color: var(--color-text);
   padding: 4px;
   border-radius: 50%;
   display: flex;
   align-items: center;
   justify-content: center;
   cursor: pointer;
   opacity: 0.7;
   transition: opacity 0.2s;
 }

.floating-book-title {
  opacity: 0;
  visibility: visible;
}

.floating-controls-area {
  position: fixed;
  bottom: 24px;
  left: 0;
  right: 0;
  height: 80px;
  z-index: 25;
  pointer-events: auto;
}

.floating-controls {
  position: fixed;
  bottom: 32px;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  align-items: center;
  background-color: rgba(var(--color-surface-rgb), 0.75);
  box-shadow: 0 2px 12px rgba(0, 0, 0, 0.15);
  border-radius: 8px;
  padding: 4px 8px;
  gap: 6px;
  z-index: 30;
  backdrop-filter: blur(10px);
  -webkit-backdrop-filter: blur(10px);
  border: 1px solid var(--color-border);
  pointer-events: auto;
  opacity: 0;
  visibility: hidden;
  transition: opacity 0.3s ease, visibility 0.3s ease;
}

.control-button {
  width: 38px;
  height: 38px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  color: var(--color-text);
  cursor: pointer;
  border-radius: 4px;
  transition: background-color 0.2s;
}

.control-button:hover {
  background-color: var(--color-background-alt);
}

.progress-display {
  font-size: 14px;
  font-weight: 500;
  padding: 0 12px;
  color: var(--color-text-secondary);
  min-width: 40px;
  text-align: center;
}

.theme-selector {
  display: flex;
  align-items: center;
  padding-left: 8px;
  margin-left: 8px;
  border-left: 1px solid var(--color-border);
  gap: 6px;
}

.theme-button {
  width: 38px;
  height: 38px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
  border-radius: 50%;
  transition: all 0.2s;
}

.theme-button:hover {
  background-color: var(--color-background-alt);
}

.theme-button.active {
  color: var(--color-primary);
  background-color: var(--color-background-alt);
}

:deep(p), :deep(h1), :deep(h2), :deep(h3), :deep(h4), :deep(h5), :deep(h6), :deep(div), :deep(span) {
  white-space: normal;
}

:deep(pre), :deep(code) {
  white-space: pre-wrap;
}

:deep(p), :deep(div) {
  word-break: break-word;
  overflow-wrap: break-word;
}

.control-button:hover,
.theme-button:hover,
.back-button:hover {
  transform: scale(1.05);
}

 ::v-deep(.slide-panel-enter-active),
 ::v-deep(.slide-panel-leave-active) {
   transition: transform 0.4s cubic-bezier(0.4, 0, 0.2, 1), opacity 0.4s ease;
 }

 ::v-deep(.slide-panel-enter-from),
 ::v-deep(.slide-panel-leave-to) {
   opacity: 0;
   transform: scale(0.97);
 }

</style>