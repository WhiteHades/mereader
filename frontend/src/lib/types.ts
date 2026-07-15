export interface Progress {
  currentLocation: number;
  currentChapterId: string | null;
  completionPercentage: number;
}

export interface BookSummary {
  id: string;
  title: string;
  author: string | null;
  progress: Progress | null;
}

export interface LibraryResponse {
  books: BookSummary[];
  total: number;
}

export interface ChapterSummary {
  id: string;
  title: string;
  order: number;
  startLocation: number;
  endLocation: number;
}

export interface BookDetail extends BookSummary {
  chapters: ChapterSummary[];
  totalLocations: number;
}

export interface CoverData {
  mimeType: string;
  data: number[];
}

export interface ChapterContent {
  chapterId: string;
  title: string;
  html: string;
  startLocation: number;
  endLocation: number;
}

export type AiState = 'ready' | 'unavailable' | 'indexing' | 'error';

export interface AiStatus {
  state: AiState;
  message?: string;
  indexedThroughLocation?: number;
  totalLocations?: number;
  available?: boolean;
  generationModelAvailable?: boolean;
  embeddingModelAvailable?: boolean;
  indexedBooks?: number;
  textOnlyBooks?: number;
  failedBooks?: number;
}

export interface SourcePassage {
  citationId: string;
  chapterId: string;
  chapterTitle: string;
  text: string;
  startLocation?: number;
  endLocation?: number;
  relevanceScore?: number;
  retrievalMethods: string[];
}

export interface ProgressBoundary {
  currentLocation: number;
  completionPercentage: number;
}

export interface AnswerResponse {
  answer: string;
  sources: SourcePassage[];
  progressBoundary: ProgressBoundary | null;
}

export type AiEvent =
  | { type: 'status'; message: string }
  | { type: 'delta'; delta: string }
  | { type: 'complete'; response: AnswerResponse }
  | { type: 'error'; message: string };

export interface ProgressPosition {
  bookId: string;
  currentLocation: number;
  currentChapterId: string;
}

export type ReaderTheme = 'light' | 'sepia' | 'dark';

export interface ReaderSettings {
  theme: ReaderTheme;
  fontSize: number;
  lineHeight: number;
  readingWidth: number;
}
