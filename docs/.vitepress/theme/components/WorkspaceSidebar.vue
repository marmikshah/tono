<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute, withBase } from "vitepress";
import Icon from "./Icon.vue";
import Logo from "./Logo.vue";
import { useStudioViewport } from "../composables/useStudioViewport";
const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ close: [] }>();
const route = useRoute();
const panel = ref<HTMLElement>();
const path = computed(() => route.path.split(/[?#]/)[0].replace(/\.html$/, "").replace(/\/$/, ""));
const { desktop } = useStudioViewport();
const links = computed(() => [
    { path: "/", label: "Library", icon: "compass", count: "" },
    { path: "/sounds", label: "Sound effects", icon: "wave", count: "384" },
    { path: "/bgm", label: "Background music", icon: "music", count: "36" },
    { path: "/create", label: desktop.value ? "Sound Studio" : "Sound demo", icon: desktop.value ? "sliders" : "play", count: "" },
    { path: "/showcase", label: "Examples", icon: "headphones", count: "" },
]);
const collections = [
    { id: "ui", title: "Interface essentials", icon: "cursor" },
    { id: "arcade", title: "Arcade & adventure", icon: "gamepad" },
    { id: "ambience", title: "Atmospheres", icon: "orbit" },
];
function active(destination: string) { return path.value === withBase(destination).replace(/\/$/, ""); }
let previousOverflow = "";
async function handleOpen(open: boolean) {
    if (open) {
        previousOverflow = document.body.style.overflow;
        document.body.style.overflow = "hidden";
        await nextTick();
        panel.value?.querySelector<HTMLElement>('[aria-current="page"], .sidebar-close')?.focus();
    } else document.body.style.overflow = previousOverflow;
}
watch(() => props.open, handleOpen);
onMounted(() => { if (props.open) void handleOpen(true); });
function keydown(event: KeyboardEvent) {
    if (!props.open) return;
    if (event.key === "Escape") { event.preventDefault(); emit("close"); }
    if (event.key !== "Tab") return;
    const elements = Array.from(panel.value?.querySelectorAll<HTMLElement>('a[href], button:not([disabled])') ?? []).filter(el => el.getClientRects().length);
    const first = elements[0], last = elements.at(-1);
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
}
onBeforeUnmount(() => { if (props.open) document.body.style.overflow = previousOverflow; });
</script>
<template>
    <div v-if="open" class="sidebar-scrim" aria-hidden="true" @click="emit('close')" />
    <aside ref="panel" class="workspace-sidebar" :class="{ 'is-open': open, 'studio-sidebar': active('/create') && desktop }" :role="open ? 'dialog' : undefined" :aria-modal="open ? true : undefined" aria-label="Workspace navigation" @keydown="keydown">
        <div class="sidebar-brand">
            <a :href="withBase('/')" aria-label="tono home" @click="emit('close')"><Logo /></a>
            <button class="icon-button sidebar-close" aria-label="Close workspace navigation" @click="emit('close')"><Icon name="close" :size="19" /></button>
        </div>
        <p class="sidebar-label">{{ desktop ? 'WORKSPACE' : 'EXPLORE' }}</p>
        <nav id="site-navigation" class="site-navigation workspace-nav" aria-label="Main navigation">
            <a v-for="link in links" :key="link.path" :href="withBase(link.path)" :aria-current="active(link.path) ? 'page' : undefined" @click="emit('close')">
                <Icon :name="link.icon" :size="18" /><span>{{ link.label }}</span><small v-if="link.count">{{ link.count }}</small>
            </a>
        </nav>
        <div v-if="active('/create') && desktop" class="studio-browser-panel"><p class="sidebar-instruments-heading">Instruments</p><div id="studio-instrument-browser" /></div>
        <div v-if="!active('/create') || !desktop" class="sidebar-collections">
            <p class="sidebar-label">COLLECTIONS</p>
            <nav aria-label="Featured collections">
                <a v-for="collection in collections" :key="collection.id" :href="withBase('/sounds?category=' + collection.id)" @click="emit('close')"><Icon :name="collection.icon" :size="16" />{{ collection.title }}</a>
            </nav>
        </div>
        <div class="sidebar-bottom">
            <div class="sidebar-resources"><a :href="withBase('/get-started/basics')"><Icon name="book" :size="15" />Docs</a><a href="https://github.com/marmikshah/tono" target="_blank" rel="noopener noreferrer"><Icon name="github" :size="15" />GitHub</a></div>
            <p class="sidebar-status"><i />Open source. Free to use.</p>
        </div>
    </aside>
</template>
<style scoped>
.workspace-sidebar { position: fixed; z-index: 70; inset: 0 auto 0 0; display: flex; flex-direction: column; width: var(--app-sidebar-width); padding: 0 12px 10px; border-right: 1px solid var(--line); background: var(--sidebar); overflow-y: auto; }
.sidebar-brand { display: flex; flex-shrink: 0; align-items: center; justify-content: space-between; min-height: var(--site-header-height); padding-inline: 5px; border-bottom: 1px solid var(--line); margin-bottom: 18px; }
.sidebar-brand :deep(.site-logo) { font-size: 24px; }
.sidebar-brand :deep(.site-logo svg) { width: 27px; }
.sidebar-close { display: none; }
.sidebar-label { margin: 7px 12px 10px !important; color: var(--muted); font-size: 10px; font-weight: 600; letter-spacing: 1.2px; }
.workspace-nav { position: static; inset: auto; display: flex; flex-direction: column; align-items: stretch; gap: 4px; margin: 0; padding: 0; border: 0; background: transparent; box-shadow: none; }
.workspace-nav a { position: relative; display: flex; gap: 10px; min-height: 44px; padding: 9px; border-radius: var(--radius-control); font-size: 13px; font-weight: 500; }
.workspace-nav a small { display: none; }
.workspace-nav a span { flex: 1; }
.workspace-nav a small { font-size: 10px; color: var(--muted); font-variant-numeric: tabular-nums; }
.workspace-nav a[aria-current="page"] { background: var(--accent-soft); color: var(--ink); }
.workspace-nav a[aria-current="page"]::before { position: absolute; left: 0; top: 12px; bottom: 12px; width: 2px; background: var(--accent-ink); content: ""; border-radius: 2px; }
.sidebar-collections { margin-top: 31px; }
.sidebar-collections nav { display: flex; flex-direction: column; }
.sidebar-collections a { display: flex; align-items: center; gap: 11px; min-height: 40px; padding: 8px 12px; border-radius: 6px; color: var(--muted); font-size: 12px; }
.sidebar-collections a:hover { background: var(--surface); color: var(--ink); }
.sidebar-bottom { flex-shrink: 0; margin-top: auto; padding-top: 24px; }
.sidebar-resources { display: flex; gap: 20px; padding: 15px 12px 5px; }
.sidebar-resources a { display: flex; align-items: center; gap: 7px; min-height: 38px; color: var(--muted); font-size: 11px; }
.sidebar-resources a:hover { color: var(--ink); }
.sidebar-status { display: flex; align-items: center; gap: 7px; padding: 4px 12px; color: var(--muted); font-size: 10px; }
.sidebar-status i { width: 5px; height: 5px; background: var(--success); border-radius: 50%; }
.studio-browser-panel { display: flex; flex: 1; flex-direction: column; min-height: 0; overflow: hidden; }
.sidebar-instruments-heading { padding: 0 3px 12px; color: var(--ink); font-size: 13px; font-weight: 600; }
#studio-instrument-browser { flex: 1; min-height: 0; display: flex; flex-direction: column; }
@media (min-width: 1100px) { .studio-sidebar > .sidebar-label, .studio-sidebar .workspace-nav, .studio-sidebar .sidebar-status { display: none; } .studio-sidebar .sidebar-bottom { padding-top: 12px; margin-top: 0; } .studio-sidebar .sidebar-resources { gap: 16px; padding-inline: 5px; } }
.sidebar-scrim { position: fixed; inset: 0; z-index: 89; background: #0009; backdrop-filter: blur(3px); }
@media (max-width: 1099px) { .workspace-sidebar { display: none; z-index: 90; width: 268px; } .workspace-sidebar.is-open { display: flex; } .sidebar-close { display: flex; width: 44px; height: 44px; } .studio-browser-panel { display: none; } .sidebar-collections a { min-height: 44px; font-size: 13px; } .sidebar-resources a { min-height: 44px; font-size: 12px; } }
@media (max-height: 750px) { .sidebar-collections { margin-top: 20px; } .sidebar-bottom { padding-top: 22px; } }
</style>
