<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRouter, withBase } from "vitepress";
import { librarySounds } from "../data/library";
import { backgroundMusic } from "../data/bgm";
import Icon from "./Icon.vue";
const emit = defineEmits<{ close: [] }>();
const router = useRouter();
const query = ref("");
const selected = ref(0);
const input = ref<HTMLInputElement>();
const dialog = ref<HTMLElement>();
const items = [...librarySounds.map(sound => ({ ...sound, kind: "Sound effect", path: "/sounds", icon: "wave" })), ...backgroundMusic.map(sound => ({ ...sound, kind: "Music loop", path: "/bgm", icon: "music" }))];
const matches = computed(() => {
    const terms = query.value.toLowerCase().trim().split(/\s+/).filter(Boolean);
    return items.filter(item => terms.every(term => [item.title, item.description, item.category, item.key, item.bpm ? item.bpm + " bpm" : "", ...item.tags, ...item.variants.map(sample => sample.label)].join(" ").toLowerCase().includes(term)));
});
const results = computed(() => query.value.trim() ? matches.value.slice(0, 10) : [items[0], items[12], items[20], items[48], ...items.slice(librarySounds.length, librarySounds.length + 3)]);
watch(query, () => { selected.value = 0; });
function destination(item: typeof items[number]) { return withBase(item.path + "?q=" + encodeURIComponent(item.title)); }
async function open(index: number) {
    const item = results.value[index];
    if (!item) return;
    emit("close");
    await router.go(destination(item));
}
function keyboard(event: KeyboardEvent) {
    if (event.key === "Escape") { event.preventDefault(); emit("close"); }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        if (results.value.length) selected.value = (selected.value + (event.key === "ArrowDown" ? 1 : -1) + results.value.length) % results.value.length;
        void nextTick(() => dialog.value?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" }));
    }
    if (event.key === "Enter" && document.activeElement === input.value) { event.preventDefault(); void open(selected.value); }
    if (event.key === "Tab") {
        const elements = Array.from(dialog.value?.querySelectorAll<HTMLElement>('input, a[href], button') ?? []).filter(el => el.getClientRects().length);
        const first = elements[0], last = elements.at(-1);
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    }
}
let previousOverflow = "";
onMounted(() => { previousOverflow = document.body.style.overflow; document.body.style.overflow = "hidden"; input.value?.focus(); });
onBeforeUnmount(() => { document.body.style.overflow = previousOverflow; });
</script>
<template>
    <div class="search-overlay" @click.self="emit('close')">
        <section ref="dialog" class="workspace-search" role="dialog" aria-modal="true" aria-label="Search sound library" @keydown="keyboard">
            <div class="search-input-row"><Icon name="search" :size="21" /><label class="sr-only" for="workspace-search-input">Search all sounds and music</label><input id="workspace-search-input" ref="input" v-model="query" type="search" role="combobox" aria-expanded="true" aria-autocomplete="list" placeholder="Search sounds, instruments, moods…" autocomplete="off" :aria-activedescendant="results.length ? 'search-result-' + selected : undefined" aria-controls="workspace-search-results" /><button class="search-close" aria-label="Close library search" @click="emit('close')"><span>Esc</span><Icon name="close" :size="17" /></button></div>
            <div class="search-results-heading"><span>{{ query.trim() ? matches.length + ' matching sounds' : 'Explore the library' }}</span><span>Effects & music</span></div>
            <div id="workspace-search-results" class="search-results" role="listbox" aria-label="Library search results">
                <a v-for="(item, index) in results" :id="'search-result-' + index" :key="item.path + item.id" role="option" :aria-selected="selected === index" :href="destination(item)" @pointermove="selected = index" @click.prevent="open(index)"><span class="search-result-icon"><Icon :name="item.icon" :size="19" /></span><span class="search-result-copy"><strong>{{ item.title }}</strong><small>{{ item.kind }} · {{ item.category }} · {{ item.variants.length }} {{ item.kind === 'Music loop' ? 'arrangements' : 'variations' }}</small></span><Icon name="arrow" :size="17" /></a>
                <div v-if="!results.length" class="search-empty"><Icon name="search" :size="27" /><strong>No sounds found</strong><p>Try a sound, instrument or mood like “laser” or “calm”.</p><button @click="query = ''; input?.focus()">Clear search</button></div>
            </div>
            <div class="search-help"><span><kbd>↑</kbd><kbd>↓</kbd> to move <kbd>↵</kbd> to open</span></div>
        </section>
    </div>
</template>
<style scoped>
.search-overlay { position: fixed; inset: 0; z-index: 100; display: flex; align-items: flex-start; justify-content: center; padding: min(14vh, 120px) 20px 20px; background: #08090dcc; backdrop-filter: blur(8px); }
.workspace-search { width: min(620px, 100%); max-height: calc(100dvh - 160px); overflow: hidden; display: flex; flex-direction: column; border: 1px solid var(--line); border-radius: var(--radius-panel); background: var(--surface); box-shadow: 0 24px 80px #0008; }
.search-input-row { display: flex; align-items: center; gap: 13px; padding: 15px 18px; border-bottom: 1px solid var(--line); color: var(--muted); }
.search-input-row input { min-width: 0; width: 100%; min-height: 36px; color: var(--ink); font: inherit; font-size: 15px; outline: none !important; }
.search-input-row input::placeholder { color: var(--muted); }
.search-input-row > svg { flex-shrink: 0; color: var(--accent-ink); }
.search-close { display: flex; align-items: center; justify-content: center; min-width: 44px; min-height: 40px; color: var(--muted); border-radius: 5px; }
.search-close span { padding: 1px 6px; border: 1px solid var(--line); border-radius: 4px; font-size: 10px; }
.search-close svg { display: none; }
.search-close:hover { background: var(--surface-hover); }
.search-results-heading { display: flex; justify-content: space-between; gap: 10px; padding: 14px 19px 8px; color: var(--muted); font-size: 11px; }
.search-results { min-height: 0; overflow-y: auto; padding: 0 8px 8px; }
.search-results > a { display: flex; align-items: center; gap: 12px; min-height: 56px; padding: 8px 12px; border-radius: var(--radius-control); }
.search-results > a[aria-selected="true"] { background: var(--surface-hover); }
.search-result-icon { display: flex; flex-shrink: 0; align-items: center; justify-content: center; width: 28px; height: 28px; color: var(--accent-ink); }
.search-result-copy { flex: 1; min-width: 0; }
.search-result-copy strong { display: block; color: var(--ink); font-size: 14px; font-weight: 550; }
.search-result-copy small { display: block; margin-top: 3px; color: var(--muted); font-size: 12px; }
.search-results > a > svg { color: var(--muted); flex-shrink: 0; }
.search-help { display: flex; align-items: center; justify-content: space-between; padding: 12px 18px; border-top: 1px solid var(--line); color: var(--muted); font-size: 10px; }
.search-help kbd { display: inline-flex; justify-content: center; min-width: 17px; margin-right: 4px; padding: 0 3px; border: 1px solid var(--line); border-radius: 3px; font: inherit; }
.search-empty { display: flex; align-items: center; flex-direction: column; gap: 12px; padding: 45px 20px; color: var(--muted); text-align: center; }
.search-empty strong { color: var(--ink); font-size: 15px; }
.search-empty p { font-size: 12px; }
.search-empty button { min-height: 44px; padding: 0 16px; color: var(--accent-ink); }
@media (max-width: 700px) { .search-overlay { padding: 12px; } .workspace-search { max-height: calc(100dvh - 24px); } .search-input-row { gap: 9px; padding: 10px 12px; } .search-input-row input { font-size: 14px; min-height: 44px; } .search-close { min-height: 44px; } .search-close span { display: none; } .search-close svg { display: block; } .search-help { display: none; } }
</style>
