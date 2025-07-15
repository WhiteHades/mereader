<template>
  <div class="app" :class="{ 'theme-dark': isDarkTheme, 'theme-sepia': isSepiaTheme }">
    <main class="main-content">
      <router-view v-slot="{ Component }">
        <transition name="fade" mode="out-in">
          <component :is="Component" />
        </transition>
      </router-view>
    </main>
  </div>
</template>

<script setup>
import { computed, onMounted } from 'vue';
import { useThemeStore } from "@/services/themeStore.js";

const themeStore = useThemeStore();
const isDarkTheme = computed(() => themeStore.theme === 'dark');
const isSepiaTheme = computed(() => themeStore.theme === 'sepia');

onMounted(() => {
  themeStore.setTheme('dark');
  applyThemeToBody(themeStore.theme);
});

function applyThemeToBody(theme) {
  document.body.className = theme === 'dark' ? 'theme-dark' :
                           theme === 'sepia' ? 'theme-sepia' : '';
}
</script>

<style>
.app {
  height: 100vh;
  display: flex;
  flex-direction: column;
  background-color: var(--color-background);
  color: var(--color-text);
  transition: background-color 0.3s, color 0.3s;
  overflow: hidden;
}

.main-content {
  flex: 1;
  overflow: hidden;
  position: relative;
}
</style>