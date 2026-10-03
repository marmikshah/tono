<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { withBase } from "vitepress";
import { useAudioPlayer } from "../composables/useAudio";
import { librarySounds } from "../data/library";
import Icon from "../components/Icon.vue";
import SoundLibraryCard from "../components/SoundLibraryCard.vue";

const player = useAudioPlayer();
const allCategory = "All sounds";
const categories = [allCategory, ...new Set(librarySounds.map((sound) => sound.category))];
const selected = ref(allCategory);
const query = ref("");
const page = ref(1);
const pageSize = 24;
const resultsHeading = ref<HTMLHeadingElement>();
const totalVariations = librarySounds.reduce((sum, sound) => sum + sound.variants.length, 0);
const terms = computed(() => query.value.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean));
const filtered = computed(() => librarySounds.filter((sound) => {
    if (selected.value !== allCategory && sound.category !== selected.value) return false;
    const text = [sound.title, sound.description, sound.category, ...sound.tags].join(" ").toLocaleLowerCase();
    return terms.value.every((term) => text.includes(term));
}));
const filteredVariations = computed(() => filtered.value.reduce((sum, sound) => sum + sound.variants.length, 0));
const pageCount = computed(() => Math.max(1, Math.ceil(filtered.value.length / pageSize)));
const paginated = computed(() => filtered.value.slice((page.value - 1) * pageSize, page.value * pageSize));
const firstResult = computed(() => filtered.value.length ? (page.value - 1) * pageSize + 1 : 0);
const lastResult = computed(() => Math.min(page.value * pageSize, filtered.value.length));
const categoryLabel = (category: string) => category === "ui" ? "UI" : category.charAt(0).toUpperCase() + category.slice(1);

watch([selected, query], () => {
    player.stop();
    page.value = 1;
});

function resetFilters() {
    selected.value = allCategory;
    query.value = "";
}

async function changePage(nextPage: number) {
    if (nextPage === page.value || nextPage < 1 || nextPage > pageCount.value) return;
    player.stop();
    page.value = nextPage;
    await nextTick();
    resultsHeading.value?.focus({ preventScroll: true });
    resultsHeading.value?.scrollIntoView({ block: "start" });
}
</script>

<template>
    <main id="main-content" class="site-container sound-library" tabindex="-1">
        <a class="back-link" :href="withBase('/')"><Icon name="arrow" :size="16" /> Back to tono</a>
        <div class="sound-library-intro">
            <div class="sound-library-copy">
                <p class="eyebrow">THE SOUND LIBRARY</p>
                <h1>Little sounds.<br /><em>Endless possibilities.</em></h1>
                <p>
                    Pick a sound, find your favorite variation, and give it a home
                    in your game. Play it here, download a WAV, or take the editable
                    recipe and make it your own.
                </p>
            </div>
            <div class="sound-library-note">
                <div class="sound-library-art" aria-hidden="true">
                    <i v-for="(height, index) in [10, 18, 30, 45, 28, 54, 72, 42, 60, 86, 50, 34, 66, 42, 25, 48, 30, 18, 12]"
                        :key="index" :style="{ height: `${height}px` }" />
                </div>
                <div class="sound-library-stat"><strong>{{ totalVariations }}</strong><span>ready-to-use variations</span></div>
                <p>{{ librarySounds.length }} sounds <b>·</b> {{ categories.length - 1 }} categories</p>
                <span class="sound-library-local"><Icon name="wave" :size="13" /> Rendered in your browser</span>
            </div>
        </div>

        <section class="sound-library-catalog" aria-label="Browse sound effects">
            <div class="sound-library-search">
                <label for="sound-library-search">Find your sound</label>
                <div class="sound-library-search-field">
                    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true">
                        <circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 4 4" />
                    </svg>
                    <input id="sound-library-search" v-model="query" type="search" placeholder="Try footsteps, laser, menu…" autocomplete="off" />
                    <button v-if="query" type="button" aria-label="Clear sound search" @click="query = ''"><Icon name="close" :size="16" /></button>
                </div>
            </div>
            <div class="sound-library-categories" role="group" aria-label="Sound categories">
                <button v-for="category in categories" :key="category" type="button" :aria-pressed="selected === category" @click="selected = category">{{ categoryLabel(category) }}</button>
            </div>
            <div class="sound-library-results">
                <h2 ref="resultsHeading" tabindex="-1">{{ categoryLabel(selected) }}</h2>
                <span role="status">{{ filtered.length }} {{ filtered.length === 1 ? "sound" : "sounds" }} <b>·</b> {{ filteredVariations }} variations</span>
            </div>
            <p v-if="player.state.error" class="sound-library-error" role="alert">{{ player.state.error }}</p>
            <div v-if="paginated.length" class="sound-library-grid">
                <SoundLibraryCard v-for="sound in paginated" :key="sound.id" :sound="sound" :player="player" />
            </div>
            <div v-else class="sound-library-empty">
                <Icon name="wave" :size="28" />
                <h3>No sounds found yet.</h3>
                <p>Try a different word or explore another category.</p>
                <button type="button" @click="resetFilters">Show all sounds <Icon name="arrow" :size="15" /></button>
            </div>
            <div v-if="filtered.length" class="sound-library-pagination">
                <span>Showing {{ firstResult }}–{{ lastResult }} of {{ filtered.length }} sounds</span>
                <nav v-if="pageCount > 1" aria-label="Sound library pages">
                    <button class="library-page-arrow" type="button" :disabled="page === 1" aria-label="Previous page" @click="changePage(page - 1)"><Icon name="arrow" :size="16" /></button>
                    <button v-for="number in pageCount" :key="number" type="button" :aria-current="page === number ? 'page' : undefined" :aria-label="`Page ${number}`" @click="changePage(number)">{{ number }}</button>
                    <button type="button" :disabled="page === pageCount" aria-label="Next page" @click="changePage(page + 1)"><Icon name="arrow" :size="16" /></button>
                </nav>
            </div>
        </section>

        <div class="sound-library-outro">
            <div>
                <p class="eyebrow">EVERY SOUND STARTS WITH A RECIPE</p>
                <h2>Find a spark. Make it yours.</h2>
                <p>Change a pitch, shape an envelope, or layer a new voice. The same tono engine turns your recipe into sound.</p>
            </div>
            <a class="button button-primary" :href="withBase('/create')">Make your own sound <Icon name="arrow" :size="16" /></a>
        </div>
    </main>
</template>

<style scoped>
.sound-library { padding-block: 45px 80px; }
.sound-library-intro {
    display: grid;
    grid-template-columns: 1fr 270px;
    align-items: center;
    gap: 70px;
    margin-block: 36px 53px;
}
.sound-library-copy h1 {
    margin-top: 16px;
    font-size: clamp(45px, 5vw, 65px);
    font-weight: 500;
    letter-spacing: -2.8px;
    line-height: 1.08;
}
.sound-library-copy > p:not(.eyebrow) {
    max-width: 535px;
    margin-top: 23px;
    color: var(--muted);
    font-size: 13px;
    line-height: 1.9;
}
.sound-library-note {
    padding: 26px 23px 24px;
    border: 1px solid #e0e5d7;
    border-radius: 10px;
    background: #f1f3e9;
    text-align: center;
}
.sound-library-art { display: flex; align-items: center; justify-content: center; gap: 5px; height: 91px; }
.sound-library-art i { width: 4px; border-radius: 3px; background: #78916a; }
.sound-library-art i:nth-child(2n) { background: #b0c19a; }
.sound-library-stat { display: flex; align-items: baseline; justify-content: center; gap: 8px; margin-top: 16px; }
.sound-library-stat strong { font-size: 35px; letter-spacing: -1.5px; font-weight: 500; }
.sound-library-stat span { font-size: 10px; color: #63725a; }
.sound-library-note > p { margin-top: 7px; color: #63725a; font-size: 10px; }
.sound-library-note b { padding-inline: 5px; font-weight: 400; }
.sound-library-local { display: flex; align-items: center; justify-content: center; gap: 6px; margin-top: 18px; padding-top: 14px; border-top: 1px solid #dce3d0; font-size: 9px; color: #596f4f; }
.sound-library-search { display: flex; align-items: center; gap: 24px; }
.sound-library-search > label { color: #43593e; font-size: 12px; font-weight: 500; }
.sound-library-search-field { display: flex; align-items: center; flex: 1; gap: 11px; max-width: 480px; padding: 0 13px; border: 1px solid #d8dfcd; background: #fffefa; border-radius: 6px; color: #728266; }
.sound-library-search-field:focus-within { outline: 2px solid #5d8d66; outline-offset: 3px; }
.sound-library-search-field input { min-width: 0; width: 100%; height: 45px; outline: none; border: 0; background: transparent; color: var(--ink); font-family: inherit; font-size: 12px; }
.sound-library-search-field input::placeholder { color: #7a8572; }
.sound-library-search-field input::-webkit-search-cancel-button { -webkit-appearance: none; }
.sound-library-search-field button { display: flex; align-items: center; justify-content: center; flex-shrink: 0; width: 30px; height: 36px; color: #697c5c; }
.sound-library-categories { display: flex; flex-wrap: wrap; gap: 7px; margin-top: 24px; padding-bottom: 24px; border-bottom: 1px solid var(--line); }
.sound-library-categories button { min-height: 35px; padding: 5px 13px; border: 1px solid transparent; border-radius: 5px; color: #5d6d54; font-size: 11px; }
.sound-library-categories button:hover { background: #edf0e4; }
.sound-library-categories button[aria-pressed="true"] { border-color: #d0dbc4; background: #e7edde; color: #375a3c; }
.sound-library-results { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 9px; margin-block: 24px 20px; }
.sound-library-results h2 { font-size: 19px; font-weight: 500; letter-spacing: -0.4px; scroll-margin-top: 110px; }
.sound-library-results h2:focus { outline: none; }
.sound-library-results span { color: #6b7663; font-size: 10px; }
.sound-library-results b { padding-inline: 6px; font-weight: 400; }
.sound-library-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 20px; }
.sound-library-error { margin-bottom: 18px; padding: 12px 16px; border: 1px solid #decaba; border-radius: 6px; background: #fbf2e9; color: #8a5038; font-size: 12px; line-height: 1.7; }
.sound-library-empty { padding: 60px 20px; border: 1px dashed #dce3d0; border-radius: 8px; text-align: center; color: #6b805f; }
.sound-library-empty h3 { margin-top: 15px; color: var(--ink); font-size: 20px; font-weight: 500; }
.sound-library-empty p { margin-top: 8px; color: var(--muted); font-size: 12px; }
.sound-library-empty button { display: inline-flex; align-items: center; gap: 7px; margin-top: 20px; min-height: 38px; padding: 5px 12px; border-radius: 5px; color: var(--forest); background: #edf2e5; font-size: 12px; }
.sound-library-pagination { display: flex; align-items: center; justify-content: space-between; gap: 20px; margin-top: 27px; }
.sound-library-pagination > span { color: #6b7663; font-size: 10px; }
.sound-library-pagination nav { display: flex; align-items: center; gap: 5px; }
.sound-library-pagination button { display: flex; align-items: center; justify-content: center; width: 36px; height: 36px; border: 1px solid transparent; border-radius: 5px; color: #5b6e51; font-size: 11px; }
.sound-library-pagination button:hover { background: #edf0e4; }
.sound-library-pagination button[aria-current="page"] { border-color: #d0dbc4; background: #e7edde; color: #375a3c; }
.sound-library-pagination button:disabled { opacity: 0.35; cursor: default; }
.library-page-arrow svg { transform: rotate(180deg); }
.sound-library-outro { display: flex; align-items: center; justify-content: space-between; gap: 38px; margin-top: 61px; padding: 31px 33px; border: 1px solid #e0e5d4; border-radius: 9px; background: #edf0e4; }
.sound-library-outro .eyebrow { font-size: 8px; letter-spacing: 1.1px; }
.sound-library-outro h2 { margin-top: 9px; font-size: 25px; font-weight: 500; letter-spacing: -0.7px; line-height: 1.3; }
.sound-library-outro p:not(.eyebrow) { max-width: 515px; margin-top: 10px; color: var(--muted); font-size: 11px; line-height: 1.8; }
.sound-library-outro .button { flex-shrink: 0; font-size: 11px; }
@media (max-width: 1100px) {
    .sound-library-intro { grid-template-columns: 1fr 245px; gap: 35px; }
    .sound-library-copy h1 { font-size: 54px; }
    .sound-library-grid { gap: 16px; }
}
@media (max-width: 900px) {
    .sound-library-intro { grid-template-columns: 1fr 205px; gap: 25px; }
    .sound-library-copy h1 { font-size: 47px; letter-spacing: -2px; }
    .sound-library-note { padding-inline: 12px; }
    .sound-library-stat { flex-direction: column; align-items: center; gap: 3px; }
    .sound-library-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .sound-library-outro { flex-direction: column; align-items: flex-start; gap: 22px; }
}
@media (max-width: 700px) {
    .sound-library { padding-block: 26px 43px; }
    .sound-library-intro { grid-template-columns: 1fr; gap: 27px; margin-block: 28px 34px; }
    .sound-library-copy h1 { font-size: clamp(43px, 8.4vw, 59px); letter-spacing: -2px; }
    .sound-library-copy > p:not(.eyebrow) { font-size: 13px; margin-top: 19px; }
    .sound-library-note { display: grid; grid-template-columns: 94px 1fr; align-items: center; column-gap: 18px; padding: 18px 20px; text-align: left; }
    .sound-library-art { grid-row: 1 / 4; width: 94px; gap: 3px; height: 85px; overflow: hidden; }
    .sound-library-art i { flex-shrink: 0; width: 2px; }
    .sound-library-stat { flex-direction: row; align-items: baseline; justify-content: flex-start; margin-top: 0; gap: 8px; }
    .sound-library-stat strong { font-size: 31px; }
    .sound-library-stat span { font-size: 9px; }
    .sound-library-note > p { margin-top: 3px; font-size: 9px; }
    .sound-library-local { justify-content: flex-start; border-top: 0; margin-top: 6px; padding-top: 0; font-size: 9px; }
    .sound-library-search { flex-direction: column; align-items: stretch; gap: 10px; }
    .sound-library-search-field { max-width: none; }
    .sound-library-categories { gap: 5px; margin-top: 19px; padding-bottom: 19px; }
    .sound-library-categories button { min-height: 38px; padding-inline: 10px; font-size: 10px; }
    .sound-library-results { margin-block: 21px 18px; }
    .sound-library-results span { font-size: 9px; }
    .sound-library-grid { grid-template-columns: 1fr; gap: 15px; }
    .sound-library-pagination { flex-direction: column; gap: 15px; margin-top: 24px; }
    .sound-library-pagination button { width: 40px; height: 40px; }
    .sound-library-outro { margin-top: 39px; padding: 25px 23px; }
    .sound-library-outro h2 { font-size: 23px; }
    .sound-library-outro p:not(.eyebrow) { font-size: 12px; }
}
@media (max-width: 370px) {
    .sound-library-note { grid-template-columns: 72px 1fr; gap: 14px; padding-inline: 15px; }
    .sound-library-art { width: 72px; }
    .sound-library-stat { flex-wrap: wrap; gap: 2px 7px; }
}
</style>
