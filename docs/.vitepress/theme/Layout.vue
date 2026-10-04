<script setup lang="ts">
import { Content, useData, useRoute } from "vitepress";
import { computed, defineAsyncComponent, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import DefaultTheme from "vitepress/theme";
import SiteHeader from "./components/SiteHeader.vue";
import SiteFooter from "./components/SiteFooter.vue";
import WorkspaceSidebar from "./components/WorkspaceSidebar.vue";
import { useStudioViewport } from "./composables/useStudioViewport";
const WorkspaceSearch = defineAsyncComponent(() => import("./components/WorkspaceSearch.vue"));
const GuideLanguage = defineAsyncComponent(() => import("./components/GuideLanguage.vue"));
const { frontmatter } = useData();
const route = useRoute();
const workspace = computed(() => ["landing", "library"].includes(frontmatter.value.layout));
const guide = computed(() => /\/(get-started|guides)(\/|$)/.test(route.path));
const { desktop } = useStudioViewport();
const editor = computed(() => desktop.value && route.path.split(/[?#]/)[0].replace(/\.html$/, "").replace(/\/$/, "").endsWith("/create"));
const navigationOpen = ref(false);
const searchOpen = ref(false);
const header = ref<InstanceType<typeof SiteHeader>>();
let previousFocus: HTMLElement | null = null;
async function closeNavigation() { navigationOpen.value = false; await nextTick(); header.value?.focusMenu(); }
async function openSearch() {
    const modal = document.querySelector<HTMLDialogElement>("dialog[open]");
    if (modal) await new Promise<void>((resolve) => {
        modal.addEventListener("close", () => resolve(), { once: true });
        modal.close();
    });
    previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    navigationOpen.value = false;
    await nextTick();
    searchOpen.value = true;
}
async function closeSearch() {
    searchOpen.value = false;
    await nextTick();
    if (previousFocus?.isConnected && previousFocus.getClientRects().length) previousFocus.focus();
    else header.value?.focusSearch();
}
function keyboard(event: KeyboardEvent) {
    if (workspace.value && (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        if (!event.repeat) void (searchOpen.value ? closeSearch() : openSearch());
    }
}
function resize() { if (window.innerWidth >= 1100) navigationOpen.value = false; }
watch(() => route.path, () => { navigationOpen.value = false; searchOpen.value = false; });
onMounted(() => { document.addEventListener("keydown", keyboard); window.addEventListener("resize", resize); });
onBeforeUnmount(() => { document.removeEventListener("keydown", keyboard); window.removeEventListener("resize", resize); });
</script>
<template>
    <div v-if="workspace" class="tono-site tono-app" :class="{ 'tono-workspace': frontmatter.layout === 'library', 'tono-editor-shell': editor }">
        <a class="skip-link" href="#main-content">Skip to content</a>
        <WorkspaceSidebar v-if="editor || navigationOpen" :open="navigationOpen" @close="closeNavigation" />
        <div class="tono-app-content">
            <SiteHeader ref="header" :navigation-open="navigationOpen" @navigation="navigationOpen ? closeNavigation() : navigationOpen = true" @search="openSearch" />
            <div class="workspace-page"><Content /></div>
            <SiteFooter v-if="!editor" />
        </div>
        <WorkspaceSearch v-if="searchOpen" @close="closeSearch" />
    </div>
    <DefaultTheme.Layout v-else>
        <template #doc-before><GuideLanguage v-if="guide" /></template>
    </DefaultTheme.Layout>
</template>
