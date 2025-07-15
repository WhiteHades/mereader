const API_BASE_URL = 'http://localhost:8000/api';

/**
 * API service for interacting with the MeReader backend
 */
export const apiService = {
  /**
   * Get all books in the library
   * @returns {Promise<Object>} List of books and total count
   */
  async getBooks() {
    try {
      const response = await fetch(`${API_BASE_URL}/books/`);
      if (!response.ok) {
        throw new Error(`Failed to fetch books: ${response.status}`);
      }
      const result = await response.json();
      return result;
    } catch (error) {
      console.error('[API] Error - getBooks:', error);
      throw error;
    }
  },

  /**
   * Upload a new book
   * @param {FormData} FormData - EPUB file to upload
   * @returns {Promise<Object>} Uploaded book information
   */
  async uploadBook(FormData) {
    try {
      const response = await fetch(`${API_BASE_URL}/books/upload`, {
        method: 'POST',
        body: FormData
      });

      if (!response.ok) {
        throw new Error(`Failed to upload book: ${response.status}`);
      }

      const result = await response.json();
      return result;
    } catch (error) {
      console.error('[API] Error - uploadBook:', error);
      throw error;
    }
  }
};