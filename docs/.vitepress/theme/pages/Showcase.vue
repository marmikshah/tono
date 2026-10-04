<script setup lang="ts">
import { computed, ref } from "vue";
import { withBase } from "vitepress";
import { useAudioPlayer } from "../composables/useAudio";
import { musicUrl, scoreUrl, tracks } from "../data/catalog";
import LibraryPlayerBar from "../components/library/LibraryPlayerBar.vue";
import Icon from "../components/Icon.vue";
const player = useAudioPlayer();
const categories = ["Everything", "Game music", "Full pieces", "Sound studies"];
const selected = ref(categories[0]);
const query = ref("");
const filtered = computed(() => {
    const words = query.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
    return tracks.filter(track => (selected.value === categories[0] || track.category === selected.value)
        && words.every(word => [track.title, track.mood, track.description, track.detail].join(" ").toLowerCase().includes(word)));
});
const current = computed(() => tracks.find(track => "music:" + track.id === player.state.activeId));
function download() {
    if (!current.value) return;
    const link = document.createElement("a");
    link.href = musicUrl(current.value);
    link.download = current.value.id + ".mp4";
    link.click();
}
</script>
<template>
    <main id="main-content" class="site-container listening-room" tabindex="-1">
        <div class="showcase-heading"><div><h1>Composition examples</h1><p>Listen to finished compositions. Explore their source or download the audio.</p></div><a :href="withBase('/bgm')">Find a music loop <Icon name="arrow" :size="16" /></a></div>
        <section class="showcase-collection" aria-labelledby="all-compositions">
            <div class="showcase-section-heading"><h2 id="all-compositions" class="sr-only">Compositions</h2><span role="status">{{ filtered.length }} {{ filtered.length === 1 ? 'composition' : 'compositions' }}</span></div>
            <div class="showcase-browser-toolbar">
                <div role="group" aria-label="Filter the listening room" class="showcase-tabs"><button v-for="category in categories" :key="category" :aria-pressed="selected === category" @click="selected = category">{{ category === 'Everything' ? 'All compositions' : category }}</button></div>
                <form class="showcase-search" role="search" @submit.prevent><Icon name="search" :size="16" /><label class="sr-only" for="showcase-search">Search the listening room</label><input id="showcase-search" v-model="query" type="search" placeholder="Search compositions…" autocomplete="off" /><button v-if="query" type="button" aria-label="Clear listening room search" @click="query = ''"><Icon name="close" :size="16" /></button></form>
            </div>
            <div v-if="filtered.length" class="showcase-list">
                <div class="composition-columns" aria-hidden="true"><span></span><span>Composition</span><span>Style</span><span>Details</span><span>Source</span></div>
                <article v-for="(track, index) in filtered" :key="track.id" class="composition-row" :class="{ 'is-playing': player.state.activeId === 'music:' + track.id && player.state.playing }">
                    <div class="composition-number"><span>{{ String(index + 1).padStart(2, '0') }}</span><button :aria-label="(player.state.activeId === 'music:' + track.id && player.state.playing ? 'Pause ' : 'Play ') + track.title" @click="player.toggle('music:' + track.id, musicUrl(track))"><Icon :name="player.state.activeId === 'music:' + track.id && player.state.playing ? 'pause' : 'play'" :size="14" /></button></div>
                    <div class="composition-title"><h3>{{ track.title }}</h3><p>{{ track.mood }}</p></div><span class="composition-style">{{ track.category }}</span><span class="composition-detail">{{ track.detail }}</span><div class="composition-actions"><a :href="scoreUrl(track)" :target="track.source ? '_blank' : undefined" :rel="track.source ? 'noopener noreferrer' : undefined" :aria-label="track.source ? 'View ' + track.title + ' source code' : 'Learn to compose songs like ' + track.title"><Icon :name="track.source ? 'code' : 'book'" :size="17" /><span>{{ track.source ? 'Source' : 'Learn' }}</span></a><a :href="musicUrl(track)" :download="track.id + '.mp4'" :aria-label="'Download ' + track.title + ' audio'"><Icon name="download" :size="17" /></a></div>
                </article>
            </div>
            <div v-else class="showcase-empty"><Icon name="search" :size="28" /><h2>No matching compositions</h2><p>Try a different title, mood or instrument.</p><button class="button button-outline" @click="query = ''; selected = categories[0]">Show all examples</button></div>
        </section>
        <p v-if="player.state.error" class="audio-error" role="alert">{{ player.state.error }}</p>
        <div class="showcase-note"><Icon name="code" :size="18" /><span>Every composition starts with a recipe.</span><a :href="withBase('/guides/songs')">Learn how to compose <Icon name="arrow" :size="15" /></a></div>
        <LibraryPlayerBar v-if="current" :player="player" :title="current.title" :subtitle="current.mood" :on-download="download" download-label="Download audio" />
    </main>
</template>
<style scoped>
.listening-room { padding-block: 28px 24px; }
.showcase-heading { display: flex; align-items: center; justify-content: space-between; gap: 24px; margin-bottom: 24px; }
.showcase-heading h1 { font-size: 28px; font-weight: 600; letter-spacing: -.8px; line-height: 1.25; }
.showcase-heading p { margin-top: 8px; color: var(--muted); font-size: 13px; }
.showcase-heading > a { display: flex; align-items: center; gap: 8px; min-height: 36px; color: var(--muted-strong); font-size: 12px; white-space: nowrap; }
.showcase-heading > a:hover { color: var(--accent-ink); }
.showcase-section-heading { display: flex; justify-content: flex-end; margin-bottom: 8px; color: var(--muted); font-size: 12px; }
.showcase-browser-toolbar { display: flex; align-items: center; justify-content: space-between; gap: 16px; padding-bottom: 16px; }
.showcase-tabs { display: flex; align-items: center; gap: 4px; }
.showcase-tabs button { min-height: 36px; padding: 0 12px; border-radius: var(--radius-control); color: var(--muted); font-size: 13px; }
.showcase-tabs button:hover { color: var(--ink); background: var(--surface-raised); }
.showcase-tabs button[aria-pressed="true"] { background: var(--surface-hover); color: var(--ink); }
.showcase-search { display: flex; align-items: center; gap: 8px; min-height: 36px; width: 280px; padding-inline: 12px; border: 1px solid var(--line); border-radius: var(--radius-control); color: var(--muted); background: var(--surface); flex-shrink: 0; }
.showcase-search:focus-within { outline: 2px solid var(--focus); outline-offset: 2px; }
.showcase-search input { min-width: 0; width: 100%; min-height: 34px; color: var(--ink); font: inherit; font-size: 13px; outline: none !important; }
.showcase-search input::placeholder { color: var(--muted); }
.showcase-search button { display: flex; align-items: center; justify-content: center; flex-shrink: 0; min-width: 30px; min-height: 31px; }
.composition-columns, .composition-row { display: grid; grid-template-columns: 40px minmax(180px, 1.3fr) minmax(95px, .6fr) minmax(140px, .8fr) 108px; align-items: center; gap: 16px; padding: 0 12px; }
.composition-columns { min-height: 32px; border-bottom: 1px solid var(--line); color: var(--muted); font-size: 12px; }
.composition-columns > span:last-child { text-align: right; }
.composition-row { min-height: 56px; border-bottom: 1px solid var(--line-soft); }
.composition-row:hover { background: var(--surface); }
.composition-row.is-playing { background: var(--accent-soft); box-shadow: inset 2px 0 var(--accent); }
.composition-number { display: flex; align-items: center; gap: 5px; color: var(--muted); font-variant-numeric: tabular-nums; font-size: 10px; }
.composition-number > span { display: none; }
.composition-number button { display: flex; align-items: center; justify-content: center; width: 36px; height: 36px; border: 1px solid transparent; border-radius: var(--radius-control); color: var(--muted-strong); background: var(--surface); }
.composition-number button:hover, .is-playing .composition-number button { color: var(--on-accent); background: var(--accent); }
.composition-title h3 { font-size: 14px; font-weight: 550; }
.composition-title p { margin-top: 2px; color: var(--muted); font-size: 12px; }
.composition-style, .composition-detail { color: var(--muted); font-size: 12px; }
.composition-actions { display: flex; align-items: center; justify-content: flex-end; gap: 5px; }
.composition-actions a { display: flex; align-items: center; justify-content: center; gap: 6px; min-width: 36px; min-height: 36px; border-radius: var(--radius-control); color: var(--muted-strong); font-size: 13px; }
.composition-actions a:hover { color: var(--accent-ink); }
.showcase-empty { padding: 60px 20px; text-align: center; color: var(--muted); }
.showcase-empty h2 { margin-top: 13px; font-size: 19px; }
.showcase-empty p { margin-top: 8px; font-size: 12px; }
.showcase-empty button { margin-top: 20px; }
.showcase-note { display: flex; align-items: center; gap: 12px; margin-top: 16px; color: var(--muted); font-size: 11px; }
.showcase-note a { display: flex; align-items: center; gap: 8px; min-height: 36px; margin-left: auto; color: var(--muted-strong); }
@media (max-width: 900px) { .composition-columns, .composition-row { grid-template-columns: 40px minmax(160px, 1fr) minmax(95px, .6fr) 108px; } .composition-detail, .composition-columns > span:nth-child(4) { display: none; } .showcase-browser-toolbar { align-items: stretch; flex-direction: column-reverse; gap: 12px; } .showcase-tabs button { min-height: 44px; } .showcase-search { width: 100%; min-height: 44px; } .showcase-search input { min-height: 42px; } }
@media (max-width: 700px) {
    .listening-room { padding-block: 20px 24px; }
    .showcase-heading { align-items: flex-start; flex-direction: column; gap: 4px; margin-bottom: 12px; }
    .showcase-heading h1 { font-size: 24px; }
    .showcase-heading p { font-size: 12px; line-height: 1.7; }
    .showcase-section-heading { margin-bottom: 5px; }
    .showcase-browser-toolbar { gap: 8px; padding-bottom: 12px; }
    .showcase-search { width: 100%; min-height: 44px; }
    .showcase-search input { font-size: 13px; min-height: 42px; }
    .showcase-search button { min-height: 42px; min-width: 32px; }
    .showcase-tabs { gap: 4px; overflow-x: auto; }
    .showcase-tabs button { flex-shrink: 0; min-height: 44px; padding-inline: 10px; font-size: 12px; }
    .composition-columns { display: none; }
    .composition-row { grid-template-columns: 44px minmax(0, 1fr) 88px; gap: 8px; padding: 7px 0; min-height: 66px; }
    .composition-number { justify-content: center; }
    .composition-number > span, .composition-style { display: none; }
    .composition-number button { width: 44px; height: 44px; }
    .composition-title h3 { font-size: 14px; }
    .composition-title p { font-size: 12px; }
    .composition-actions { gap: 0; }
    .composition-actions a { min-width: 44px; min-height: 44px; }
    .composition-actions a span { display: none; }
    .showcase-note { flex-wrap: wrap; gap: 8px; font-size: 10px; }
    .showcase-note a { margin-left: 0; flex-basis: 100%; }
}
</style>
