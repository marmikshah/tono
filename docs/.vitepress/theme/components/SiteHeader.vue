<script setup lang="ts">
import { computed, ref } from "vue";
import { useRoute, withBase } from "vitepress";
import Icon from "./Icon.vue";
import Logo from "./Logo.vue";
import { useStudioViewport } from "../composables/useStudioViewport";
const props = defineProps<{ navigationOpen: boolean }>();
const emit = defineEmits<{ navigation: []; search: [] }>();
const route = useRoute();
const menuButton = ref<HTMLButtonElement>();
const searchButton = ref<HTMLButtonElement>();
const path = computed(() => route.path.split(/[?#]/)[0].replace(/\.html$/, "").replace(/\/$/, ""));
const inStudio = computed(() => path.value === withBase("/create"));
const { desktop } = useStudioViewport();
const links = computed(() => [
    { url: "/", label: "Library" },
    { url: "/create", label: desktop.value ? "Studio" : "Sound demo" },
    { url: "/showcase", label: "Examples" },
    { url: "/get-started/basics", label: "Docs" },
]);
function active(url: string) {
    if (url === "/") return ["/", "/sounds", "/bgm"].some(value => path.value === withBase(value).replace(/\/$/, ""));
    return path.value === withBase(url).replace(/\/$/, "");
}
defineExpose({ focusMenu: () => menuButton.value?.focus(), focusSearch: () => searchButton.value?.focus() });
</script>
<template>
    <header class="site-header workspace-topbar" :class="{ 'studio-topbar': inStudio && desktop }">
        <div class="workspace-topbar-inner">
            <button ref="menuButton" class="icon-button workspace-menu" :aria-label="props.navigationOpen ? 'Close navigation' : 'Open navigation'" :aria-expanded="props.navigationOpen" aria-controls="site-navigation" @click="emit('navigation')"><Icon :name="props.navigationOpen ? 'close' : 'menu'" :size="22" /></button>
            <a class="header-brand" :href="withBase('/')" aria-label="tono home"><Logo /></a>
            <span v-if="inStudio && desktop" class="header-studio-label">Sound Studio</span>
            <nav class="header-navigation" aria-label="Main navigation"><a v-for="link in links" :key="link.url" :href="withBase(link.url)" :aria-current="active(link.url) ? 'page' : undefined">{{ link.label }}</a></nav>
            <div class="workspace-topbar-actions">
                <button ref="searchButton" class="workspace-search-trigger" aria-label="Search library" aria-haspopup="dialog" @click="emit('search')"><Icon name="search" :size="19" /><span>Search library</span><kbd>Ctrl K</kbd></button>
                <a v-if="!inStudio" class="button workspace-create" :href="withBase('/create')" :aria-label="desktop ? 'Create a sound' : 'Try the sound demo'"><Icon :name="desktop ? 'plus' : 'play'" :size="16" /><span>{{ desktop ? 'Create a sound' : 'Try demo' }}</span></a>
            </div>
        </div>
    </header>
</template>
<style scoped>
.workspace-topbar { height: var(--site-header-height); background: var(--sidebar); border-bottom-color: var(--line); }
.workspace-topbar-inner { display: flex; align-items: center; gap: 28px; width: calc(100% - 40px); height: 100%; margin-inline: auto; }
.header-brand { display: flex; flex-shrink: 0; }
.header-brand :deep(.site-logo) { font-size: 24px; letter-spacing: -1.3px; }
.header-brand :deep(.site-logo svg) { width: 21px; height: 25px; }
.workspace-menu { display: none; }
.header-navigation { display: flex; align-items: center; gap: 4px; height: 100%; margin-left: 8px; }
.header-navigation a { position: relative; display: flex; align-items: center; min-height: 32px; padding: 0 12px; border-radius: var(--radius-control); color: var(--muted); font-size: 13px; font-weight: 500; white-space: nowrap; }
.header-navigation a:hover, .header-navigation a[aria-current="page"] { color: var(--ink); }
.header-navigation a[aria-current="page"] { color: var(--ink); background: var(--surface-hover); }
.workspace-topbar-actions { display: flex; align-items: center; gap: 16px; margin-left: auto; }
.workspace-search-trigger { display: flex; align-items: center; gap: 9px; min-height: 32px; padding: 0 10px; border: 1px solid var(--line); border-radius: var(--radius-control); background: var(--surface); color: var(--muted); font-size: 13px !important; }
.workspace-search-trigger:hover { color: var(--ink); }
.workspace-search-trigger kbd { margin-left: 22px; padding: 1px 4px; border-radius: 3px; background: var(--surface-raised); color: var(--muted); font: inherit; font-size: 10px; }
.workspace-create { min-height: 32px; padding: 0 12px; gap: 7px; border-radius: var(--radius-control); color: var(--on-accent); background: var(--accent); font-size: 13px; font-weight: 600; }
.workspace-create:hover { background: var(--accent-hover); }
.header-studio-label { color: var(--muted-strong); font-size: 15px; font-weight: 500; }
@media (min-width: 1100px) { .studio-topbar .header-brand, .studio-topbar .header-studio-label { display: none; } }
@media (max-width: 1099px) { .header-navigation, .header-studio-label { display: none; } .workspace-menu { display: flex; flex-shrink: 0; width: 44px; height: 44px; margin-left: -8px; } .workspace-topbar-inner { gap: 14px; } .workspace-search-trigger, .workspace-create { min-height: 44px; } }
@media (max-width: 700px) { .workspace-topbar-inner { width: calc(100% - 24px); gap: 9px; } .header-brand :deep(.site-logo) { font-size: 24px; } .header-brand :deep(.site-logo svg) { width: 21px; height: 25px; } .workspace-topbar-actions { gap: 7px; } .workspace-search-trigger { justify-content: center; width: 44px; min-height: 44px; padding: 0; } .workspace-search-trigger span, .workspace-search-trigger kbd { display: none; } .workspace-create { min-height: 44px; padding: 0 10px; gap: 5px; } }
@media (hover: none), (pointer: coarse) { .header-navigation a, .workspace-search-trigger, .workspace-create { min-height: 44px; } }
</style>
