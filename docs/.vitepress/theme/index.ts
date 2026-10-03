import type { Theme } from "vitepress";
import DefaultTheme from "vitepress/theme";
import { defineAsyncComponent } from "vue";
import Layout from "./Layout.vue";
import Home from "./pages/Home.vue";
import Showcase from "./pages/Showcase.vue";
import "./styles/site.css";

export default {
  extends: DefaultTheme,
  Layout,
  enhanceApp({ app }) {
    app.component("TonoHome", Home);
    app.component("TonoShowcase", Showcase);
    app.component("TonoSoundLibrary", defineAsyncComponent(() => import("./pages/SoundLibrary.vue")));
    app.component("TonoBgmLibrary", defineAsyncComponent(() => import("./pages/BgmLibrary.vue")));
    app.component("TonoSoundStudio", defineAsyncComponent(() => import("./pages/SoundStudio.vue")));
  },
} satisfies Theme;
