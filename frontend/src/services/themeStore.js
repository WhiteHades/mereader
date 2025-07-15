import { defineStore } from 'pinia';
import { ref } from 'vue';

export const useThemeStore = defineStore('theme', () => {
  const theme = ref('dark');

  function setTheme(newTheme) {
    if (['dark', 'light', 'sepia'].includes(newTheme)) {
      theme.value = newTheme;
      applyThemeToBody(newTheme);
    }
  }

  function applyThemeToBody(theme) {
    document.body.className = theme === 'dark' ? 'theme-dark' :
                             theme === 'sepia' ? 'theme-sepia' : '';
  }

  setTheme('dark');

  return {
    theme,
    setTheme
  };
});