<template>
  <div class="library-view" :class="{ 'theme-dark': isDarkTheme, 'theme-sepia': isSepiaTheme }">
    <div class="library-header">
      <div class="app-branding">
        <img src="@/assets/mereader_icon.png" alt="MeReader" class="app-icon" />
        <h1>MeReader</h1>
      </div>
      <button class="btn" @click="openBookDialog">
        <PlusCircleIcon class="icon" />
        Add Book
      </button>
    </div>

    <!-- loading -->
    <div v-if="isLoading" class="loading-container">
      <div class="loading-spinner"></div>
      <p>Loading your library...</p>
    </div>

    <!-- empty -->
    <div v-else-if="books.length === 0" class="library-state empty-state">
      <div class="empty-illustration">
        <svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="round" stroke-linejoin="round">
          <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"></path>
          <path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"></path>
        </svg>
      </div>
      <h2>Your library is empty</h2>
      <p>Add EPUB books to start reading with MeReader's AI-enhanced experience</p>
      <button class="btn" @click="openBookDialog">Add Your First Book</button>
    </div>

    <!-- book grid -->
    <div v-else class="book-grid">
      <div
        v-for="book in books"
        :key="book.id"
        class="book-card"
        @click="openBook(book.id)"
        @mousemove="handleTilt($event, book.id)"
        @mouseleave="resetTilt(book.id)"
        :style="cardTiltStyles[book.id]"
      >
        <div class="cover">
          <img v-if="coverImage(book)" :src="coverImage(book)" :alt="book.title" class="cover-img" />
           <div v-else class="cover-placeholder" :style="{ backgroundColor: getRandomColor(book.title) }">
            {{ getBookInitials(book.title) }}
          </div>
          <div v-if="book.progress && book.progress > 0" class="progress-indicator">
            {{ Math.round(book.progress) }}%
          </div>
        </div>
        <div class="info">
          <h3 class="title">{{ book.title }}</h3>
          <p class="author">{{ book.author }}</p>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { open } from '@tauri-apps/api/dialog';
import { readBinaryFile } from '@tauri-apps/api/fs';
import { basename } from "@tauri-apps/api/path";
import { useThemeStore } from "@/services/themeStore.js";
import { apiService } from "@/services/apiService.js";
import { PlusCircleIcon } from '@heroicons/vue/24/outline'

// state
const router = useRouter();
const books = ref([]);
const isLoading = ref(true);
const themeStore = useThemeStore();
const cardTiltStyles = ref({})

// computed
const isDarkTheme = computed(() => themeStore.theme === 'dark');
const isSepiaTheme = computed(() => themeStore.theme === 'sepia');

onMounted(async () => {
  await fetchBooks();
});

function coverImage(book) {
  if (!book.cover_path) return null;
  return `http://localhost:8000/api/books/cover/${book.id}`;
}

async function fetchBooks() {
  isLoading.value = true;
  try {
    const response = await apiService.getBooks();

    books.value = response.books.map(book => ({
      id: book.id,
      title: book.title,
      author: book.author,
      cover_path: book.cover_path,
      progress: book.completion_percentage || 0
    }));

  } catch (error) {
    console.error('Error fetching books:', error);
    alert('Could not load your library. Please try again.');
  } finally {
    isLoading.value = false;
  }
}

async function openBookDialog() {
  try {
    const selected = await open({
      multiple: false,
      filters: [{ name: 'EPUB', extensions: ['epub'] }],
    });

    if (!selected) return;

    isLoading.value = true;

    const fileBytes = await readBinaryFile(selected);
    const fileName = await basename(selected);

    const uint8Array = new Uint8Array(fileBytes);
    const epubFile = new File([uint8Array], fileName, {
      type: 'application/epub+zip',
    });

    const formData = new FormData();
    formData.append('file', epubFile);

    await apiService.uploadBook(formData);

    await fetchBooks();
  } catch (error) {
    console.error('Error uploading book:', error.message || error);
    alert(`Failed to upload book: ${error.message || 'Unknown error'}`);
  } finally {
    isLoading.value = false;
  }
}

function openBook(bookId) {
  router.push({ name: 'reader', params: { id: bookId } });
}

function getRandomColor(text) {
  let hash = 0;
  for (let i = 0; i < text.length; i++) {
    hash = text.charCodeAt(i) + ((hash << 5) - hash);
  }

  const colors = [
    '#0071e3', // blue
    '#5e5ce6', // indigo
    '#bf5af2', // purple
    '#ff375f', // pink
    '#ff9f0a', // orange
    '#30d158', // green
    '#64d2ff', // light blue
    '#ff453a', // red
  ];

  return colors[Math.abs(hash) % colors.length];
}

function getBookInitials(title) {
  if (!title) return '?';

  const words = title.split(' ');
  if (words.length === 1) {
    return title.substring(0, 2).toUpperCase();
  } else {
    return (words[0][0] + words[1][0]).toUpperCase();
  }
}

function handleTilt(event, bookId) {
  const card = event.currentTarget;
  const rect = card.getBoundingClientRect();
  const centerX = rect.left + rect.width / 2;
  const centerY = rect.top + rect.height / 2;
  const offsetX = event.clientX - centerX;
  const offsetY = event.clientY - centerY;

  const rotateX = (-offsetY / 70);
  const rotateY = (offsetX / 70);

  cardTiltStyles.value[bookId] = {
    transform: `perspective(600px) rotateX(${rotateX}deg) rotateY(${rotateY}deg) scale(1.03)`,
    transition: 'transform 0.1s ease-out',
  };
}

function resetTilt(bookId) {
  cardTiltStyles.value[bookId] = {
    transform: 'perspective(600px) rotateX(0deg) rotateY(0deg) scale(1)',
    transition: 'transform 0.3s ease',
  };
}

</script>

<style scoped>
.library-view {
  height: 100%;
  padding: 1.5rem;
  overflow-y: auto;
  background-color: var(--color-background);
  color: var(--color-text);
}

.library-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  border-radius: 8px;
  padding: 16px;
  margin-bottom: 2rem;
  background-color: transparent;
  backdrop-filter: none;
  -webkit-backdrop-filter: none;
  box-shadow: none;
  border: none;
}

.library-header h1 {
  margin: 0;
  font-size: 1.75rem;
  font-weight: 600;
  color: var(--color-text);
}

.loading-container {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: calc(100vh - 150px);
  width: 100%;
  color: var(--color-text);
  gap: 1rem;
}

.library-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  padding: 4rem 2rem;
  height: calc(100% - 100px);
  background-color: rgba(var(--color-surface-rgb), 0.75);
  backdrop-filter: blur(10px);
  -webkit-backdrop-filter: blur(10px);
  border-radius: 8px;
  border: 1px solid var(--color-border);
  margin-bottom: 2rem;
}

.empty-illustration {
  margin-bottom: 1.5rem;
  color: var(--color-text-secondary);
  opacity: 0.5;
}

.library-state h2 {
  margin-bottom: 0.75rem;
  font-weight: 600;
  font-size: 1.25rem;
  color: var(--color-text);
}

.library-state p {
  margin-bottom: 2rem;
  color: var(--color-text-secondary);
  max-width: 400px;
}

.book-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
  gap: 1.5rem;
  margin-bottom: 2rem;
  padding: 1.5rem;
  border-radius: 8px;
  background-color: transparent;
  backdrop-filter: none;
  -webkit-backdrop-filter: none;
  box-shadow: none;
  border: none;
}

.book-card {
  position: relative;
  display: flex;
  flex-direction: column;
  height: 100%;
  background-color: rgba(var(--color-surface-rgb), 0.85);
  border-radius: 8px;
  overflow: hidden;
  box-shadow: var(--box-shadow);
  cursor: pointer;
  transition: all var(--transition-normal);
  border: 1px solid var(--color-border);
}

.book-card:hover {
  box-shadow: 0 12px 24px var(--color-shadow);
}

.cover {
  position: relative;
  width: 100%;
  padding-top: 150%;
  overflow: hidden;
  border-radius: inherit;
}

.cover-placeholder {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 2rem;
  font-weight: 600;
  color: var(--color-text);
  transition: transform var(--transition-normal);
  border-radius: inherit;
}

.book-card:hover .cover-placeholder {
  transform: scale(1.05);
}

.book-card:hover .cover-img {
  border-radius: inherit;
}

.info {
  padding: 1rem;
  flex-grow: 1;
  display: flex;
  flex-direction: column;
}

.title {
  margin: 0 0 0.25rem;
  font-size: 0.875rem;
  font-weight: 600;
  display: -webkit-box;
  line-clamp: 2;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  line-height: 1.3;
  color: var(--color-text);
}

.author {
  margin: 0;
  font-size: 0.75rem;
  color: var(--color-text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cover-img {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
  transition: transform var(--transition-normal);
}

.app-branding {
  display: flex;
  align-items: center;
  gap: 12px;
}

.app-icon {
  width: 32px;
  height: 32px;
  object-fit: contain;
}

.progress-indicator {
  position: absolute;
  bottom: 8px;
  right: 8px;
  padding: 3px 8px;
  border-radius: 6px;
  border: 1px solid var(--color-border);
  background-color: rgba(var(--color-surface-rgb), 0.85);
  color: var(--color-text);
  font-size: 0.75rem;
  font-weight: 600;
  opacity: 0.9;
  box-shadow: 0 2px 4px rgba(0, 0, 0, 0.2);
  backdrop-filter: blur(10px);
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

.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
  padding: 0.75rem 1rem;
  font-weight: 500;
  font-size: var(--font-size-sm);
  border-radius: 8px;
  color: var(--color-text);
  cursor: pointer;
  transition: all var(--transition-normal);
  box-shadow: var(--box-shadow);
  background-color: rgba(var(--color-surface-rgb), 0.85);
  border: 1px solid var(--color-border);
}

.btn:hover {
  transform: translateY(-2px);
  box-shadow: 0 8px 16px var(--color-shadow);
}

.btn svg {
  stroke: currentColor;
}

.icon {
  width: 20px;
  height: 20px;
  stroke-width: 2;
  color: var(--color-text);
}

</style>