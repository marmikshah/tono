<script setup lang="ts">
import { defineAsyncComponent } from "vue";
import { withBase } from "vitepress";
import { useStudioViewport } from "../composables/useStudioViewport";
import Icon from "../components/Icon.vue";
const { desktop, ready } = useStudioViewport();
const DesktopSoundStudio = defineAsyncComponent(() => import("./DesktopSoundStudio.vue"));
const SoundDemo = defineAsyncComponent(() => import("./SoundDemo.vue"));
</script>
<template>
    <DesktopSoundStudio v-if="ready && desktop" />
    <SoundDemo v-else-if="ready" />
    <main v-else id="main-content" class="site-container studio-loading" tabindex="-1"><Icon name="wave" :size="28" /><h1>Sound previews</h1><p role="status">Loading your sound preview…</p><a :href="withBase('/sounds')">Browse sound effects<Icon name="arrow" :size="16" /></a></main>
</template>
<style scoped>
.studio-loading { padding-block: 48px; }
.studio-loading > svg { color: var(--accent); }
.studio-loading h1 { margin-top: 16px; font-size: 24px; font-weight: 600; }
.studio-loading p { margin-block: 10px 20px; color: var(--muted); font-size: 14px; }
.studio-loading a { display: inline-flex; align-items: center; gap: 8px; min-height: 44px; color: var(--accent-ink); font-size: 14px; }
</style>
