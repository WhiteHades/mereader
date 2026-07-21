import { Channel, invoke } from '@tauri-apps/api/core';
import type {
  AiEvent,
  AiStatus,
  AnswerResponse,
  BookDetail,
  BookSummary,
  ChapterContent,
  CoverData,
  LibraryResponse,
  Progress,
} from './types';

export function listBooks(): Promise<LibraryResponse> {
  return invoke<LibraryResponse>('list_books');
}

export function importBook(): Promise<BookSummary | null> {
  return invoke<BookSummary | null>('import_book');
}

export function getBook(bookId: string): Promise<BookDetail> {
  return invoke<BookDetail>('get_book', { bookId });
}

export function getCover(bookId: string): Promise<CoverData> {
  return invoke<CoverData>('get_cover', { bookId });
}

export function getChapter(bookId: string, chapterId: string): Promise<ChapterContent> {
  return invoke<ChapterContent>('get_chapter', { bookId, chapterId });
}

export function getChapterAsset(bookId: string, assetName: string): Promise<CoverData> {
  return invoke<CoverData>('get_chapter_asset', { bookId, assetName });
}

export function updateProgress(
  bookId: string,
  currentLocation: number,
  currentChapterId: string,
): Promise<Progress> {
  return invoke<Progress>('update_progress', {
    bookId,
    currentLocation,
    currentChapterId,
  });
}

export function deleteBook(bookId: string): Promise<boolean> {
  return invoke<boolean>('delete_book', { bookId });
}

export function getAiStatus(bookId: string): Promise<AiStatus> {
  return invoke<AiStatus>('get_ai_status', { bookId });
}

export function reindexBook(bookId: string): Promise<AiStatus> {
  return invoke<AiStatus>('reindex_book', { bookId });
}

export function askBook(
  bookId: string,
  requestId: string,
  question: string,
  onEvent: (event: AiEvent) => void,
): Promise<AnswerResponse> {
  const channel = new Channel<AiEvent>();
  channel.onmessage = onEvent;
  return invoke<AnswerResponse>('ask_book', { bookId, requestId, question, onEvent: channel });
}

export function cancelAiRequest(requestId: string): Promise<boolean> {
  return invoke<boolean>('cancel_ai_request', { requestId });
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === 'string' && error) return error;
  if (
    typeof error === 'object' &&
    error !== null &&
    'message' in error &&
    typeof error.message === 'string' &&
    error.message.length > 0
  ) return error.message;
  return 'The reader core returned an unknown error.';
}
