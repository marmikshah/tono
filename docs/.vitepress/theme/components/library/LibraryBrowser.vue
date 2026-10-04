<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute, withBase } from "vitepress";
import { useAudioPlayer } from "../../composables/useAudio";
import { useStudioViewport } from "../../composables/useStudioViewport";
import { sampleId } from "../../data/catalog";
import type { LibrarySound } from "../../data/library";
import Icon from "../Icon.vue";
import SoundLibraryCard from "../SoundLibraryCard.vue";
import LibraryPlayerBar from "./LibraryPlayerBar.vue";
import LibrarySoundInspector from "./LibrarySoundInspector.vue";

const props = withDefaults(defineProps<{ sounds: LibrarySound[]; kind: "sfx" | "bgm"; homepage?: boolean }>(), { homepage: false });
const emit = defineEmits<{ "kind-change": [kind: "sfx" | "bgm"] }>();
const route = useRoute();
const player = useAudioPlayer();
const { desktop: studioDesktop } = useStudioViewport();
const query = ref("");
const category = ref("");
const savedOnly = ref(false);
const savedKeys = ref<string[]>([]);
const view = ref<"rows" | "cards">("rows");
const variants = ref<Record<string, number>>({});
const page = ref(1);
const resultsHeading = ref<HTMLHeadingElement>();
const selectedSoundId = ref<string>();
const inlineDetailsId = ref<string>();
const desktopInspector = ref(false);
let inspectorMedia: MediaQueryList | undefined;
const savedStorageKey = "tono.saved-sounds.v1";
const viewStorageKey = `tono.library-view.v3.${props.kind}`;
const pageSize = 24;
let mounted = false;

const isMusic = computed(() => props.kind === "bgm");
const title = computed(() => props.homepage ? "Sounds" : isMusic.value ? "Background music" : "Sound effects");
const unit = computed(() => isMusic.value ? "theme" : "sound");
const variationsLabel = computed(() => isMusic.value ? "arrangements" : "variations");
const categories = computed(() => [...new Set(props.sounds.map((sound) => sound.category))]);
const totalVariants = computed(() => props.sounds.reduce((sum, sound) => sum + sound.variants.length, 0));
const categoryLabel = (value: string) => value === "ui" ? "UI" : value.charAt(0).toUpperCase() + value.slice(1);
const savedKey = (sound: LibrarySound) => `${props.kind}:${sound.id}`;
const isSaved = (sound: LibrarySound) => savedKeys.value.includes(savedKey(sound));
const savedCount = computed(() => props.sounds.filter(isSaved).length);
const words = computed(() => query.value.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean));
const searched = computed(() => props.sounds.filter((sound) => {
    if (savedOnly.value && !isSaved(sound)) return false;
    const text = [sound.title, sound.description, sound.category, sound.key, sound.bpm ? `${sound.bpm} bpm` : "", ...sound.tags, ...sound.variants.map((sample) => sample.label)].join(" ").toLocaleLowerCase();
    return words.value.every((word) => text.includes(word));
}));
const categoryCounts = computed(() => Object.fromEntries(categories.value.map((value) => [value, searched.value.filter((sound) => sound.category === value).length])));
const filtered = computed(() => searched.value.filter((sound) => !category.value || sound.category === category.value));
const filteredVariants = computed(() => filtered.value.reduce((sum, sound) => sum + sound.variants.length, 0));
const pageCount = computed(() => Math.max(1, Math.ceil(filtered.value.length / pageSize)));
const paginated = computed(() => filtered.value.slice((page.value - 1) * pageSize, page.value * pageSize));
const selectedSound = computed(() => paginated.value.find((sound) => sound.id === selectedSoundId.value));
const firstResult = computed(() => filtered.value.length ? (page.value - 1) * pageSize + 1 : 0);
const lastResult = computed(() => Math.min(page.value * pageSize, filtered.value.length));
const resultTitle = computed(() => category.value ? categoryLabel(category.value) : savedOnly.value ? `Saved ${isMusic.value ? "music" : "sounds"}` : isMusic.value ? "All music" : "All sounds");
const activeSelection = computed(() => {
    for (const sound of props.sounds) {
        const sample = sound.variants.find((take) => sampleId(take) === player.state.activeId);
        if (sample) return { sound, sample };
    }
    return undefined;
});
const activeSubtitle = computed(() => activeSelection.value
    ? `${activeSelection.value.sample.label || "Original"} · ${activeSelection.value.sound.bpm ? `${activeSelection.value.sound.bpm} BPM · ${activeSelection.value.sound.key}` : categoryLabel(activeSelection.value.sound.category)}`
    : "");
const descriptions: Record<string, string> = {
    ui: "Buttons, menus and notifications.", arcade: "Pickups, power-ups and playful game cues.",
    "sci-fi": "Signals, lasers and sounds for futuristic worlds.", motion: "Footsteps, movement and quick transitions.",
    impact: "Hits, crashes and explosive moments.", mechanical: "Switches, motors and moving machinery.",
    ambience: "Atmospheres and seamless environmental loops.", percussion: "Drums, hits and rhythmic accents.",
    tonal: "Chimes, bells and short musical accents.", menu: "Welcoming loops for title screens and menus.",
    puzzle: "Light rhythms for thinking and solving.", cozy: "Warm melodies for a quieter world.",
    exploration: "Open melodies for discovering somewhere new.", fantasy: "Magical themes for imaginary worlds.",
    action: "Driving rhythms for battles and big moments.", suspense: "Tense, restrained loops for the unknown.",
    chill: "Relaxed beats for taking things slowly.", adventure: "Bright themes for setting out on a quest.",
};
const resultDescription = computed(() => category.value ? descriptions[category.value] || ""
    : savedOnly.value ? "Your bookmarks on this device. No account needed."
    : isMusic.value ? "Pick Original, Calm or Drive. Every arrangement loops on the beat." : "Choose a variation, audition it, and download a WAV.");

function isCurrentLibrary() {
    const normalize = (path: string) => path.replace(/\/+$/, "").replace(/\.html$/, "");
    const path = normalize(window.location.pathname);
    if (props.homepage) {
        const urlKind = new URLSearchParams(window.location.search).get("kind") === "bgm" ? "bgm" : "sfx";
        return path === normalize(withBase("/")) && urlKind === props.kind;
    }
    return path === normalize(withBase(isMusic.value ? "/bgm" : "/sounds"));
}
function readUrl() {
    // popstate fires before the router unmounts the departing library.
    if (!mounted || !isCurrentLibrary()) return;
    const params = new URLSearchParams(window.location.search);
    query.value = (params.get("q") || "").slice(0, 200);
    const value = params.get("category") || "";
    category.value = categories.value.includes(value) ? value : "";
    savedOnly.value = params.get("saved") === "1";
    syncUrl();
}
function syncUrl() {
    if (!mounted || !isCurrentLibrary()) return;
    const url = new URL(window.location.href);
    for (const [key, value] of [["q", query.value], ["category", category.value], ["saved", savedOnly.value ? "1" : ""]]) {
        if (value) url.searchParams.set(key, value); else url.searchParams.delete(key);
    }
    if (url.href !== window.location.href) window.history.replaceState(window.history.state, "", url);
}
function readSaved() {
    try {
        const value: unknown = JSON.parse(localStorage.getItem(savedStorageKey) || "[]");
        savedKeys.value = Array.isArray(value) ? value.filter((key): key is string => typeof key === "string" && key.length < 128).slice(0, 512) : [];
    } catch { savedKeys.value = []; }
}
function toggleSaved(sound: LibrarySound) {
    const key = savedKey(sound);
    savedKeys.value = isSaved(sound) ? savedKeys.value.filter((value) => value !== key) : [...savedKeys.value, key];
    try { localStorage.setItem(savedStorageKey, JSON.stringify(savedKeys.value)); } catch { /* Bookmarks still work for this visit when storage is unavailable. */ }
}
function changeView(next: "rows" | "cards") {
    view.value = next;
    try { localStorage.setItem(viewStorageKey, next); } catch { /* Keep the current view for this visit. */ }
}
function changeVariant(sound: LibrarySound, index: number) {
    if (!Number.isInteger(index) || !sound.variants[index]) return;
    const previous = sound.variants[variants.value[sound.id] || 0];
    const auditioning = player.state.activeId === sampleId(previous) && (player.state.playing || Boolean(player.state.loadingId));
    variants.value = { ...variants.value, [sound.id]: index };
    if (auditioning) void player.toggleSample(sound.variants[index]);
}
function refreshSelection() {
    if (!paginated.value.some((sound) => sound.id === selectedSoundId.value)) {
        selectedSoundId.value = desktopInspector.value ? paginated.value[0]?.id : undefined;
        inlineDetailsId.value = undefined;
    }
    if (!paginated.value.some((sound) => sound.id === inlineDetailsId.value)) inlineDetailsId.value = undefined;
}
function updateInspectorMode() {
    const next = inspectorMedia?.matches || false;
    if (next !== desktopInspector.value) inlineDetailsId.value = undefined;
    desktopInspector.value = next;
    refreshSelection();
}
function selectSound(sound: LibrarySound) {
    selectedSoundId.value = sound.id;
    if (!desktopInspector.value) inlineDetailsId.value = inlineDetailsId.value === sound.id ? undefined : sound.id;
}
function selectAuditionedSound(sound: LibrarySound) {
    selectedSoundId.value = sound.id;
    if (!desktopInspector.value && inlineDetailsId.value !== sound.id) inlineDetailsId.value = undefined;
}
async function closeInlineDetails(sound: LibrarySound) {
    inlineDetailsId.value = undefined;
    await nextTick();
    document.getElementById(`library-${sound.id}-select`)?.focus({ preventScroll: true });
}
function resetFilters() { query.value = ""; category.value = ""; savedOnly.value = false; }
async function changePage(next: number) {
    if (next === page.value || next < 1 || next > pageCount.value) return;
    page.value = next;
    await nextTick();
    resultsHeading.value?.focus({ preventScroll: true });
    resultsHeading.value?.scrollIntoView({ block: "start", behavior: "instant" });
}
function onStorage(event: StorageEvent) { if (event.key === savedStorageKey) readSaved(); }
watch([query, category, savedOnly], () => { page.value = 1; syncUrl(); });
watch(pageCount, (count) => { if (page.value > count) page.value = count; });
watch(paginated, refreshSelection);
watch(() => route.query, readUrl);
onMounted(() => {
    mounted = true;
    readSaved();
    try { const stored = localStorage.getItem(viewStorageKey); if (stored === "rows" || stored === "cards") view.value = stored; } catch { /* Default to rows. */ }
    readUrl();
    inspectorMedia = window.matchMedia("(min-width: 1200px)");
    inspectorMedia.addEventListener("change", updateInspectorMode);
    updateInspectorMode();
    window.addEventListener("popstate", readUrl);
    window.addEventListener("storage", onStorage);
});
onBeforeUnmount(() => { mounted = false; inspectorMedia?.removeEventListener("change", updateInspectorMode); window.removeEventListener("popstate", readUrl); window.removeEventListener("storage", onStorage); });
</script>

<template>
    <main id="main-content" class="site-container library-browser" :class="isMusic ? 'bgm-library' : 'sound-library'" tabindex="-1">
        <header class="library-browser-header">
            <div class="library-heading-group">
                <div class="library-title"><h1>{{ title }}</h1><p>{{ sounds.length }} {{ isMusic ? homepage ? 'themes' : 'original themes' : homepage ? 'recipes' : 'sound recipes' }} <span>·</span> {{ totalVariants }} {{ homepage && !isMusic ? 'variants' : variationsLabel }}</p></div>
                <div v-if="homepage" class="library-kind-tabs" role="group" aria-label="Sound library"><button type="button" :aria-pressed="kind === 'sfx'" @click="emit('kind-change', 'sfx')">Effects</button><button type="button" :aria-pressed="kind === 'bgm'" @click="emit('kind-change', 'bgm')">Music</button></div>
            </div>
            <div class="library-search-toolbar">
                <div class="library-search">
                    <label class="sr-only" :for="`${kind}-library-search`">{{ isMusic ? 'Find your soundtrack' : 'Find your sound' }}</label>
                    <div class="library-search-field">
                        <Icon name="search" :size="18" />
                        <input :id="`${kind}-library-search`" v-model="query" type="search" maxlength="200" :placeholder="isMusic ? 'Search music, mood or key…' : 'Search sounds, e.g. laser…'" autocomplete="off" />
                        <button v-if="query" type="button" aria-label="Clear search" @click="query = ''"><Icon name="close" :size="17" /></button>
                    </div>
                </div>
                <button class="library-saved-filter" type="button" :aria-pressed="savedOnly" :aria-label="`Saved ${isMusic ? 'music' : 'sounds'}, ${savedCount}`" @click="savedOnly = !savedOnly"><Icon :name="savedOnly ? 'star-filled' : 'star'" :size="17" /><span>Saved</span><b>{{ savedCount }}</b></button>
            </div>
        </header>
        <p class="library-mobile-hint">Play a sound. Download WAV to use it.</p>
        <section class="library-catalog" :class="{ 'has-inspector': desktopInspector && selectedSound }" :aria-label="`Browse ${title.toLowerCase()}`">
            <aside class="library-browser-rail">
                <p class="library-rail-heading">{{ isMusic ? 'Moods' : 'Categories' }}</p>
                <div class="library-categories" :class="isMusic ? 'bgm-filters' : 'sound-library-categories'" role="group" :aria-label="isMusic ? 'Music moods' : 'Sound categories'">
                    <button type="button" :aria-pressed="!category" @click="category = ''">{{ isMusic ? 'All moods' : 'All sounds' }} <span>{{ searched.length }}</span></button>
                    <button v-for="value in categories" :key="value" type="button" :aria-pressed="category === value" @click="category = value">{{ categoryLabel(value) }} <span>{{ categoryCounts[value] }}</span></button>
                </div>
            </aside>
            <div class="library-assets">
                <div class="library-results">
                    <div class="library-results-copy"><h2 ref="resultsHeading" tabindex="-1">{{ category || savedOnly ? resultTitle : isMusic ? 'Music' : 'Samples' }}</h2><p role="status">{{ filtered.length }} {{ unit }}{{ filtered.length === 1 ? '' : 's' }} · {{ filteredVariants }} {{ variationsLabel }}</p></div>
                    <div class="library-view" role="group" aria-label="Library view">
                        <button type="button" :aria-pressed="view === 'rows'" aria-label="List view" title="List view" @click="changeView('rows')"><Icon name="list" :size="19" /></button>
                        <button type="button" :aria-pressed="view === 'cards'" aria-label="Card view" title="Card view" @click="changeView('cards')"><Icon name="grid" :size="18" /></button>
                    </div>
                </div>
                <p v-if="category || savedOnly" class="library-filter-description">{{ resultDescription }}</p>
                <p v-if="player.state.error" class="sound-library-error" role="alert">{{ player.state.error }}</p>
                <div v-if="view === 'rows' && paginated.length" class="library-column-headings" aria-hidden="true"><span></span><span>{{ isMusic ? 'Track' : 'Sound' }}</span><span class="library-column-wave">Waveform</span><span>Length</span><span></span></div>
                <div v-if="paginated.length" class="library-items" :class="view === 'rows' ? 'library-list' : 'library-grid'">
                    <template v-for="sound in paginated" :key="sound.id">
                        <SoundLibraryCard :sound="sound" :player="player" :variant="variants[sound.id] || 0" :saved="isSaved(sound)" :compact="view === 'rows'" :selected="desktopInspector ? selectedSoundId === sound.id : inlineDetailsId === sound.id" :inline-details="!desktopInspector" @select="selectSound(sound)" @audition="selectAuditionedSound(sound)" />
                        <LibrarySoundInspector v-if="!desktopInspector && inlineDetailsId === sound.id" class="library-inline-inspector" :sound="sound" :player="player" :variant="variants[sound.id] || 0" :saved="isSaved(sound)" inline @variant="changeVariant(sound, $event)" @save="toggleSaved(sound)" @close="closeInlineDetails(sound)" />
                    </template>
                </div>
                <div v-else class="sound-library-empty">
                    <Icon :name="savedOnly ? 'star' : 'search'" :size="28" />
                    <h3>{{ savedOnly && !savedCount ? 'Build your own shortlist' : 'No matches found' }}</h3>
                    <p>{{ savedOnly && !savedCount ? 'Open a sound’s details to save it on this device.' : 'Try a different search or category.' }}</p>
                    <button type="button" @click="resetFilters">{{ isMusic ? 'Show all music' : 'Show all sounds' }} <Icon name="arrow" :size="16" /></button>
                </div>
                <div v-if="filtered.length" class="library-pagination">
                    <span>Showing {{ firstResult }}–{{ lastResult }} of {{ filtered.length }} {{ unit }}{{ filtered.length === 1 ? '' : 's' }}</span>
                    <nav v-if="pageCount > 1" :aria-label="`${title} pages`">
                        <button type="button" :disabled="page === 1" aria-label="Previous page" @click="changePage(page - 1)"><Icon class="library-page-back" name="arrow" :size="17" /></button>
                        <button v-for="number in pageCount" :key="number" type="button" :aria-current="page === number ? 'page' : undefined" :aria-label="`Page ${number}`" @click="changePage(number)">{{ number }}</button>
                        <button type="button" :disabled="page === pageCount" aria-label="Next page" @click="changePage(page + 1)"><Icon name="arrow" :size="17" /></button>
                    </nav>
                </div>
            </div>
            <aside v-if="desktopInspector && selectedSound" class="library-desktop-inspector" aria-label="Selected sound"><LibrarySoundInspector :key="selectedSound.id" :sound="selectedSound" :player="player" :variant="variants[selectedSound.id] || 0" :saved="isSaved(selectedSound)" @variant="changeVariant(selectedSound!, $event)" @save="toggleSaved(selectedSound!)" /></aside>
        </section>
        <p class="library-footnote"><span>Original sounds. MIT licensed. Yours to use.</span><a :href="withBase('/create')">{{ studioDesktop ? 'Open Sound Studio' : 'Open sound demo' }} <Icon name="arrow" :size="14" /></a></p>
        <LibraryPlayerBar v-if="activeSelection" :player="player" :title="activeSelection.sound.title" :subtitle="activeSubtitle" :on-download="() => player.downloadSample(activeSelection!.sample)" />
    </main>
</template>

<style scoped>
.library-browser { --library-columns: 44px minmax(150px, .85fr) minmax(120px, 1.15fr) 58px 44px; --library-areas: "play identity wave duration download"; --library-gap: 8px; padding-block: 24px 32px; }
.library-browser-header { display: flex; align-items: center; justify-content: space-between; gap: 24px; margin-bottom: 24px; }
.library-mobile-hint { display: none; }
.library-heading-group { display: flex; align-items: center; flex-shrink: 0; gap: 24px; }
.library-title h1 { color: var(--ink); font-size: 28px; font-weight: 600; letter-spacing: -.8px; line-height: 1.2; }
.library-title p { margin-top: 6px; color: var(--muted); font-size: 12px; font-variant-numeric: tabular-nums; }
.library-title p span { margin-inline: 3px; }
.library-kind-tabs { display: flex; align-items: center; gap: 4px; padding: 4px; border: 1px solid var(--line); border-radius: 7px; background: var(--surface); }
.library-kind-tabs button { min-height: 44px; padding: 8px 14px; border-radius: 4px; color: var(--muted); font-size: 13px; }
.library-kind-tabs button[aria-pressed="true"] { color: var(--ink); background: var(--surface-raised); }
.library-search-toolbar { display: flex; align-items: center; gap: 8px; width: min(670px, 60%); }
.library-search { flex: 1; min-width: 0; }
.library-search-field { display: flex; align-items: center; gap: 10px; height: 46px; padding-inline: 12px; border: 1px solid var(--line); border-radius: 6px; color: var(--muted); background: var(--surface); }
.library-search-field:focus-within { border-color: var(--focus); }
.library-search-field input { width: 100%; min-width: 0; height: 44px; border: 0; outline: none; color: var(--ink); background: transparent; font-family: inherit; font-size: 14px; }
.library-search-field input::placeholder { color: var(--muted); }
.library-search-field input::-webkit-search-cancel-button { -webkit-appearance: none; }
.library-search-field button { display: flex; align-items: center; justify-content: center; flex-shrink: 0; width: 44px; height: 44px; margin-right: -10px; color: var(--muted-strong); }
.library-saved-filter { display: inline-flex; align-items: center; justify-content: center; gap: 8px; min-height: 44px; padding: 8px 12px; border-radius: 5px; color: var(--muted-strong); font-size: 13px; white-space: nowrap; }
.library-saved-filter:hover { color: var(--ink); background: var(--surface-hover); }
.library-saved-filter[aria-pressed="true"] { color: var(--accent-ink); background: var(--accent-soft); }
.library-saved-filter b { color: var(--muted); font-size: 12px; font-weight: 400; font-variant-numeric: tabular-nums; }
.library-catalog { display: grid; grid-template-columns: 192px minmax(0, 1fr); align-items: start; gap: 16px; }
.library-catalog.has-inspector { grid-template-columns: 192px minmax(0, 1fr) 280px; }
.library-browser-rail { position: sticky; top: calc(var(--site-header-height, 56px) + 16px); align-self: start; max-height: calc(100svh - var(--site-header-height, 56px) - var(--preview-player-height, 0px) - 128px); overflow-y: auto; padding: 12px 8px 8px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface); scrollbar-width: thin; scrollbar-color: var(--line) transparent; }
.library-rail-heading { margin-bottom: 8px; padding-inline: 8px; color: var(--muted); font-size: 12px; font-weight: 400; }
.library-categories { display: flex; flex-direction: column; gap: 4px; }
.library-categories button { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-height: 44px; padding: 8px; border-radius: 4px; color: var(--muted-strong); font-size: 14px; text-align: left; white-space: nowrap; }
.library-categories button:hover { color: var(--ink); background: var(--surface-hover); }
.library-categories button[aria-pressed="true"] { color: var(--ink); background: var(--surface-raised); }
.library-categories span { color: var(--muted); font-size: 12px; font-variant-numeric: tabular-nums; }
.library-assets { overflow: hidden; min-width: 0; border: 1px solid var(--line); border-radius: 8px; background: var(--surface); }
.library-results { display: flex; align-items: center; justify-content: space-between; gap: 16px; min-height: 48px; padding: 3px 8px 3px 16px; border-bottom: 1px solid var(--line); }
.library-results-copy { display: flex; align-items: baseline; flex-wrap: wrap; gap: 4px 12px; }
.library-results h2 { color: var(--ink); font-size: 14px; font-weight: 500; scroll-margin-top: calc(var(--site-header-height, 56px) + 16px); }
.library-results h2:focus { outline: none; }
.library-results p { color: var(--muted); font-size: 12px; }
.library-filter-description { margin: 12px 16px; color: var(--muted); font-size: 12px; }
.library-view { display: flex; flex-shrink: 0; gap: 4px; }
.library-view button { display: inline-flex; align-items: center; justify-content: center; width: 44px; height: 44px; border-radius: 4px; color: var(--muted); }
.library-view button:hover { color: var(--ink); background: var(--surface-hover); }
.library-view button[aria-pressed="true"] { color: var(--ink); background: var(--surface-raised); }
.library-column-headings { display: grid; grid-template-columns: var(--library-columns); gap: var(--library-gap); padding: 9px 8px; color: var(--muted); background: var(--surface); font-size: 12px; }
.library-list { display: grid; }
.library-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 16px; padding: 16px; }
.library-inline-inspector { grid-column: 1 / -1; margin-inline: 8px; }
.library-desktop-inspector { position: sticky; top: calc(var(--site-header-height, 56px) + 16px); max-height: calc(100svh - var(--site-header-height, 56px) - var(--preview-player-height, 0px) - 128px); overflow-y: auto; min-width: 0; border: 1px solid var(--line); border-radius: 8px; background: var(--surface); scrollbar-width: thin; scrollbar-color: var(--line) transparent; }
.library-desktop-inspector :deep(.library-sound-inspector) { border: 0; border-radius: 0; }
.sound-library-error { margin: 12px 16px; padding: 12px; border-left: 2px solid var(--danger); color: var(--danger); background: var(--danger-soft); font-size: 14px; line-height: 1.6; }
.sound-library-empty { display: flex; flex-direction: column; align-items: center; padding: 64px 20px; color: var(--muted); text-align: center; }
.sound-library-empty h3 { margin-top: 16px; color: var(--ink); font-size: 22px; font-weight: 600; letter-spacing: -.5px; }
.sound-library-empty p { margin-top: 8px; color: var(--muted-strong); font-size: 14px; }
.sound-library-empty button { display: inline-flex; align-items: center; gap: 8px; min-height: 44px; margin-top: 24px; padding: 10px 16px; border-radius: 5px; color: var(--on-accent); background: var(--accent); font-size: 13px; font-weight: 500; }
.sound-library-empty button:hover { background: var(--accent-hover); }
.library-pagination { display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: 16px; }
.library-pagination > span { color: var(--muted); font-size: 12px; }
.library-pagination nav { display: flex; align-items: center; gap: 4px; }
.library-pagination button { display: flex; align-items: center; justify-content: center; width: 44px; height: 44px; border-radius: 4px; color: var(--muted-strong); font-size: 13px; }
.library-pagination button:hover { color: var(--ink); background: var(--surface-hover); }
.library-pagination button[aria-current="page"] { color: var(--ink); background: var(--surface-raised); }
.library-pagination button:disabled { opacity: .35; cursor: default; }
.library-page-back { transform: rotate(180deg); }
.library-footnote { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-top: 24px; color: var(--muted); font-size: 12px; }
.library-footnote a { display: inline-flex; align-items: center; gap: 8px; min-height: 44px; color: var(--muted-strong); }
button:focus-visible, a:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
@media (max-width: 900px) { .library-browser-header { flex-direction: column; align-items: stretch; gap: 16px; } .library-search-toolbar { width: 100%; } .library-catalog { display: block; } .library-browser-rail { position: static; padding: 0; border: 0; border-radius: 0; margin-bottom: 16px; background: transparent; } .library-rail-heading { display: none; } .library-categories { flex-direction: row; gap: 8px; overflow-x: auto; padding-bottom: 4px; scrollbar-width: thin; scrollbar-color: var(--line) transparent; } .library-categories button { flex-shrink: 0; justify-content: flex-start; min-width: 44px; min-height: 44px; padding-inline: 10px; } .library-categories span { display: none; } .library-pagination { flex-direction: column; gap: 12px; } }
@media (max-width: 700px) { .library-browser { --library-columns: 44px minmax(0, 1fr) 48px 44px; --library-areas: "play identity duration download"; padding-block: 16px 24px; } .library-browser-header { gap: 16px; margin-bottom: 16px; } .library-heading-group { justify-content: space-between; gap: 12px; } .library-kind-tabs { gap: 1px; padding: 3px; } .library-kind-tabs button { min-width: 54px; padding-inline: 8px; } .library-search-toolbar { gap: 4px; } .library-search-field svg { flex-shrink: 0; } .library-saved-filter { padding-inline: 10px; } .library-saved-filter span { display: none; } .library-results { gap: 8px; padding-left: 12px; } .library-results-copy { gap: 3px 8px; } .library-results p { width: 100%; } .library-filter-description { display: none; } .library-column-headings { display: none; } .library-grid { grid-template-columns: 1fr; padding: 12px; } .library-footnote { align-items: flex-start; flex-direction: column; gap: 4px; } }
@media (max-width: 900px) { .library-browser-rail { max-height: none; overflow-y: visible; } }
@media (max-width: 1099px) { .library-mobile-hint { display: block; margin: -8px 0 12px; color: var(--muted-strong); font-size: 13px; line-height: 1.5; } }
</style>
