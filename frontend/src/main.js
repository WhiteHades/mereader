import { createApp } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";
import { createPinia } from "pinia";
import App from "./App.vue";
import "./style.css";

import LibraryView from "./views/LibraryView.vue";
import ReaderView from "./views/ReaderView.vue";

const pinia = createPinia();

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    {
      path: "/",
      name: "library",
      component: LibraryView,
    },
    {
      path: "/reader/:id",
      name: "reader",
      component: ReaderView,
      props: true,
      beforeLeave: async (to, from, next) => {
        try {
          if (from.name === "reader") {
            const readerComponent = from.matched[0].instances.default;
            if (
              readerComponent &&
              typeof readerComponent.saveProgressBeforeLeaving === "function"
            ) {
              await readerComponent.saveProgressBeforeLeaving();
            }
          }
        } catch (error) {
          console.error("Error during navigation guard:", error);
        }
        next();
      },
    },
  ],
});

router.beforeEach(async (to, from, next) => {
  try {
    if (from.name === "reader") {
      const readerComponent = from.matched[0].instances.default;
      if (
        readerComponent &&
        typeof readerComponent.saveProgressBeforeLeaving === "function"
      ) {
        await readerComponent.saveProgressBeforeLeaving();
      }
    }
  } catch (error) {
    console.error("Error during navigation guard:", error);
  }
  next();
});

const app = createApp(App);

app.use(router);
app.use(pinia);

app.mount("#app");
