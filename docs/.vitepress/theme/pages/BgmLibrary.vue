<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { withBase } from "vitepress";
import { backgroundMusic } from "../data/bgm";
import { useAudioPlayer } from "../composables/useAudio";
import Icon from "../components/Icon.vue";
import SoundLibraryCard from "../components/SoundLibraryCard.vue";

const player = useAudioPlayer();
const category = ref("All moods");
const query = ref("");
const categories = ["All moods", ...new Set(backgroundMusic.map((track) => track.category))];
const arrangements = backgroundMusic.reduce((total, track) => total + track.variants.length, 0);
const label = (value: string) => value.charAt(0).toUpperCase() + value.slice(1);
const filtered = computed(() => {
    const words = query.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
    return backgroundMusic.filter((track) =>
        (category.value === "All moods" || track.category === category.value)
        && words.every((word) => [track.title, track.description, track.category, track.key, ...track.tags].join(" ").toLowerCase().includes(word)),
    );
});
watch([category, query], () => player.stop());
</script>

<template>
    <main id="main-content" class="site-container bgm-library" tabindex="-1">
        <a class="back-link" :href="withBase('/')"><Icon name="arrow" :size="16" /> Back to tono</a>
        <div class="bgm-intro">
            <div>
                <p class="eyebrow">BACKGROUND MUSIC</p>
                <h1>A mood for<br /><em>every little world.</em></h1>
                <p>Original themes for menus, exploring, quiet moments and the next big challenge.
                    Choose an arrangement, press play, and take a seamless loop into your game.</p>
            </div>
            <aside class="bgm-note">
                <Icon name="layers" :size="36" />
                <strong>{{ arrangements }}</strong>
                <span>musical loops · {{ backgroundMusic.length }} original themes</span>
                <p>Original, Calm &amp; Drive arrangements.<br />WAV, editable recipe and song score.</p>
            </aside>
        </div>
        <section aria-label="Browse background music">
            <div class="bgm-search">
                <label for="bgm-search">Find your soundtrack</label>
                <input id="bgm-search" v-model="query" type="search" placeholder="Try cozy, battle, space…" autocomplete="off" />
            </div>
            <div class="filter-row bgm-filters">
                <div class="filter-tabs" role="group" aria-label="Music moods">
                    <button v-for="mood in categories" :key="mood" type="button" :aria-pressed="category === mood" @click="category = mood">{{ label(mood) }}</button>
                </div>
                <span class="result-count" role="status">{{ filtered.length }} themes</span>
            </div>
            <p v-if="player.state.error" class="bgm-error" role="alert">{{ player.state.error }}</p>
            <div v-if="filtered.length" class="bgm-grid">
                <SoundLibraryCard v-for="track in filtered" :key="track.id" :sound="track" :player="player" />
            </div>
            <div v-else class="bgm-empty">
                <p>No themes match this search.</p>
                <button class="button button-outline" type="button" @click="query = ''; category = 'All moods'">Show all music</button>
            </div>
        </section>
        <p class="collection-note"><Icon name="wave" :size="14" /> Loops stay on the beat and keep playing until paused. Rendered in your browser. Free to use under MIT.</p>
        <div class="bgm-outro">
            <div><h2>Have your own melody in mind?</h2><p>Add instruments, write a few notes, and make something that belongs to your world.</p></div>
            <a class="button button-primary" :href="withBase('/create')">Make your own music <Icon name="arrow" :size="16" /></a>
            <a class="text-link" :href="withBase('/showcase')">More listening <Icon name="arrow" :size="15" /></a>
        </div>
    </main>
</template>

<style scoped>
.bgm-library { padding-block: 45px 75px; }
.bgm-intro { display: grid; grid-template-columns: minmax(0, 1fr) 270px; gap: 65px; align-items: center; margin-block: 40px 45px; }
.bgm-intro h1 { font-size: clamp(45px, 5vw, 64px); font-weight: 500; letter-spacing: -2.7px; line-height: 1.08; margin-top: 17px; }
.bgm-intro > div > p:not(.eyebrow) { max-width: 535px; margin-top: 23px; color: var(--muted); line-height: 1.9; font-size: 13px; }
.bgm-note { display: flex; flex-direction: column; align-items: center; padding: 27px 20px; background: #edf0e4; border: 1px solid #dfe5d7; border-radius: 10px; text-align: center; color: #617b55; }
.bgm-note strong { display: block; margin-top: 13px; font-size: 43px; line-height: 1.1; font-weight: 500; letter-spacing: -1.5px; color: var(--ink); }
.bgm-note span { margin-top: 8px; font-size: 11px; }
.bgm-note p { margin-top: 15px; border-top: 1px solid #d8e0cc; padding-top: 15px; font-size: 10px; line-height: 1.8; }
.bgm-search { display: flex; align-items: center; gap: 20px; margin-bottom: 22px; }
.bgm-search label { font-size: 12px; color: var(--forest); }
.bgm-search input { height: 45px; width: min(470px, 100%); min-width: 0; padding: 0 14px; border: 1px solid #d8dfcd; border-radius: 6px; background: #fffefa; font: inherit; font-size: 12px; color: var(--ink); }
.bgm-search input:focus-visible { outline: 2px solid #5d8d66; outline-offset: 3px; }
.bgm-filters { padding-bottom: 21px; border-bottom: 1px solid var(--line); margin-bottom: 23px; }
.bgm-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 20px; }
.bgm-error { padding: 12px 15px; margin-bottom: 20px; border: 1px solid #decaba; border-radius: 6px; color: #8a5038; background: #fbf2e9; font-size: 12px; line-height: 1.7; }
.bgm-empty { padding: 50px 20px; text-align: center; border: 1px dashed #dce3d0; border-radius: 8px; }
.bgm-empty p { color: var(--muted); font-size: 13px; margin-bottom: 20px; }
.bgm-outro { display: flex; align-items: center; flex-wrap: wrap; gap: 22px; margin-top: 45px; padding: 28px; background: #edf0e4; border: 1px solid #dfe5d7; border-radius: 9px; }
.bgm-outro > div { flex: 1; min-width: min(100%, 250px); }
.bgm-outro h2 { font-size: 25px; letter-spacing: -0.7px; font-weight: 500; }
.bgm-outro p { margin-top: 9px; color: var(--muted); font-size: 11px; line-height: 1.8; }
.bgm-outro .button { font-size: 11px; }
.bgm-outro .text-link { font-size: 11px; }
@media (max-width: 950px) { .bgm-intro { gap: 28px; grid-template-columns: minmax(0, 1fr) 220px; } .bgm-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
@media (max-width: 700px) {
    .bgm-library { padding-block: 26px 45px; }
    .bgm-intro { grid-template-columns: 1fr; gap: 25px; margin-block: 28px 35px; }
    .bgm-intro h1 { font-size: clamp(43px, 9vw, 59px); letter-spacing: -2px; }
    .bgm-note { padding: 22px; }
    .bgm-search { flex-direction: column; align-items: stretch; gap: 10px; }
    .bgm-search input { width: 100%; }
    .bgm-grid { grid-template-columns: 1fr; gap: 15px; }
    .bgm-outro { padding: 24px; }
    .bgm-outro h2 { font-size: 23px; }
}
</style>
