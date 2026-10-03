<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { AudioPlayer } from "../composables/useAudio";
import { sampleId, sampleUrl } from "../data/catalog";
import type { LibrarySound } from "../data/library";
import Icon from "./Icon.vue";
import Waveform from "./Waveform.vue";

const props = defineProps<{ sound: LibrarySound; player: AudioPlayer }>();
const variant = ref(0);
const sample = computed(() => props.sound.variants[variant.value]);
const id = computed(() => sampleId(sample.value));
const active = computed(() => props.player.state.activeId === id.value);
const playing = computed(() => active.value && props.player.state.playing);
const loading = computed(() => props.player.state.loadingId === id.value);
const downloading = computed(() => props.player.state.downloadId === id.value);
const categoryLabel = computed(() => props.sound.category === "ui" ? "UI" : props.sound.category.charAt(0).toUpperCase() + props.sound.category.slice(1));
const categoryIcon = computed(() => ({ ui: "ui-confirm", arcade: "coin", "sci-fi": "laser", motion: "footstep", impact: "impact", mechanical: "layers", ambience: "wave", percussion: "impact", tonal: "wave" })[props.sound.category] || "wave");
const accent = computed(() => {
    const colors = ["#688262", "#a28353", "#75869f", "#a47576", "#8576a1", "#62817c"];
    const index = [...props.sound.category].reduce((sum, letter) => sum + letter.charCodeAt(0), 0);
    return colors[index % colors.length];
});

watch(variant, (_, previous) => {
    const previousId = sampleId(props.sound.variants[previous]);
    if (props.player.state.activeId === previousId || props.player.state.loadingId === previousId) {
        props.player.stop();
    }
});
</script>

<template>
    <article
        class="library-sound-card"
        :class="{ 'is-playing': playing, 'is-rendering': loading }"
        :style="{ '--library-accent': accent }"
        :aria-labelledby="`library-${sound.id}-title`"
    >
        <div class="library-card-top">
            <span class="library-card-symbol"><Icon :name="categoryIcon" :size="20" /></span>
            <span class="library-card-category">{{ categoryLabel }}</span>
            <span v-if="sound.looping" class="library-loop" title="Repeats until paused">Loop</span>
        </div>
        <h3 :id="`library-${sound.id}-title`">{{ sound.title }}</h3>
        <p class="library-card-description">{{ sound.description }}</p>
        <p v-if="sound.bpm" class="library-score-meta">{{ sound.bpm }} BPM <b>·</b> {{ sound.key }} <b>·</b> {{ sound.bars }} bars</p>
        <div class="library-card-wave">
            <Waveform
                compact
                :values="sample.waveform"
                :progress="active ? player.state.progress : 0"
            />
        </div>
        <div class="library-variant-row">
            <label :for="`library-${sound.id}-variant`">Variation</label>
            <select
                :id="`library-${sound.id}-variant`"
                v-model="variant"
                :aria-label="`${sound.title} variation`"
            >
                <option
                    v-for="(take, index) in sound.variants"
                    :key="sampleId(take)"
                    :value="index"
                >
                    {{ take.label || `Variation ${index + 1}` }}
                </option>
            </select>
        </div>
        <div class="library-card-meta">
            <span>{{ sample.duration.toFixed(2) }}s <b>·</b> {{ sample.sampleRate / 1000 }} kHz</span>
            <span>Seed {{ sample.seed }}</span>
        </div>
        <div class="library-card-actions">
            <button
                class="library-play"
                type="button"
                :aria-busy="loading"
                :aria-label="`${loading ? 'Cancel rendering' : playing ? 'Pause' : 'Play'} ${sound.title}, ${sample.label || `variation ${variant + 1}`}`"
                @click="player.toggleSample(sample)"
            >
                <span v-if="loading" class="library-spinner" aria-hidden="true" />
                <Icon v-else :name="playing ? 'pause' : 'play'" :size="14" />
                {{ loading ? "Cancel" : playing ? "Pause" : "Play" }}
            </button>
            <button
                class="library-wav"
                type="button"
                :disabled="Boolean(player.state.downloadId)"
                :aria-busy="downloading"
                :aria-label="`Download ${sound.title}, ${sample.label || `variation ${variant + 1}`}, as WAV`"
                @click="player.downloadSample(sample)"
            >
                <span v-if="downloading" class="library-spinner" aria-hidden="true" />
                <Icon v-else name="download" :size="15" />
                {{ downloading ? "Rendering…" : "WAV" }}
            </button>
            <a
                class="library-recipe"
                :href="sampleUrl(sample.source)"
                :download="sample.source.split('/').pop()"
                :aria-label="`Download the editable recipe for ${sound.title}, ${sample.label || `variation ${variant + 1}`}`"
                title="Download the editable sound recipe"
            >
                <Icon name="code" :size="15" /> Recipe
            </a>
            <a v-if="sample.score" class="library-score" :href="sampleUrl(sample.score)" :download="sample.score"
                :aria-label="`Download the editable song score for ${sound.title}, ${sample.label}`"><Icon name="layers" :size="14" /> Score</a>
        </div>
    </article>
</template>

<style scoped>
.library-sound-card {
    min-width: 0;
    padding: 23px 23px 17px;
    border: 1px solid #dfe3d7;
    border-radius: 10px;
    background: #fffefa;
    transition: border-color 0.2s, box-shadow 0.2s;
}
.library-sound-card:hover,
.library-sound-card.is-playing,
.library-sound-card.is-rendering {
    border-color: #a9b59c;
    box-shadow: 0 6px 18px -12px #324e4255;
}
.library-card-top {
    display: flex;
    align-items: center;
    gap: 10px;
}
.library-card-symbol {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border: 1px solid color-mix(in srgb, var(--library-accent) 20%, white);
    border-radius: 8px;
    color: var(--library-accent);
    background: color-mix(in srgb, var(--library-accent) 9%, white);
}
.library-card-category {
    color: #65735e;
    font-size: 10px;
}
.library-loop {
    margin-left: auto;
    padding: 2px 6px;
    border: 1px solid #dce3d2;
    border-radius: 4px;
    color: #5c7152;
    font-size: 9px;
    line-height: 17px;
}
.library-sound-card h3 {
    margin-top: 17px;
    font-size: 18px;
    font-weight: 550;
    letter-spacing: -0.45px;
    line-height: 1.4;
}
.library-card-description {
    min-height: 40px;
    margin-top: 6px;
    color: var(--muted);
    font-size: 11px;
    line-height: 1.8;
}
.library-score-meta { margin-top: 8px; color: #5b7254; font-size: 10px; }
.library-score-meta b { margin-inline: 5px; font-weight: 400; }
.library-score { display: inline-flex; align-items: center; gap: 4px; color: #667b5b; font-size: 10px; }
.library-card-actions { flex-wrap: wrap; }
.library-card-wave {
    padding-block: 12px 5px;
    color: var(--library-accent);
}
.library-card-wave :deep(.waveform) {
    display: block;
    width: 100%;
    height: 54px;
}
.library-card-wave :deep(rect) {
    fill: color-mix(in srgb, var(--library-accent) 55%, white);
}
.library-card-wave :deep(.wave-played) {
    fill: var(--forest);
}
.library-card-wave :deep(.wave-baseline) {
    stroke: #e1e6d9;
}
.library-variant-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-top: 11px;
}
.library-variant-row label {
    color: #626e5b;
    font-size: 10px;
}
.library-variant-row select {
    min-width: 111px;
    min-height: 36px;
    padding: 5px 25px 5px 10px;
    border: 1px solid #dfe4d6;
    border-radius: 5px;
    background-color: #f7f8f1;
    color: #435b3d;
    font-family: inherit;
    font-size: 11px;
    cursor: pointer;
}
.library-variant-row select:focus-visible {
    outline: 2px solid #5d8d66;
    outline-offset: 3px;
}
.library-card-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 4px 10px;
    margin-top: 12px;
    color: #707966;
    font-family: var(--vp-font-family-mono);
    font-size: 8px;
    line-height: 18px;
}
.library-card-meta b {
    padding-inline: 3px;
    font-weight: 400;
}
.library-card-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    padding-top: 13px;
    border-top: 1px solid #e8ebdf;
}
.library-card-actions button,
.library-recipe {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-height: 36px;
    border-radius: 5px;
    font-size: 10px;
    line-height: 1.2;
    white-space: nowrap;
}
.library-play {
    min-width: 78px;
    padding: 7px 12px;
    background: #edf2e5;
    color: #385b42;
}
.library-play:hover {
    background: #e0e9d7;
}
.library-wav {
    padding: 7px;
    color: #55694c;
}
.library-wav:hover,
.library-recipe:hover {
    background: #edf2e5;
}
.library-recipe {
    margin-left: auto;
    padding: 7px;
    color: #69775f;
}
.library-card-actions button:disabled {
    cursor: wait;
    opacity: 0.6;
}
.library-spinner {
    width: 12px;
    height: 12px;
    border: 1.5px solid #a9b89f;
    border-top-color: #385b42;
    border-radius: 50%;
    animation: library-spin 0.7s linear infinite;
}
@keyframes library-spin {
    to { transform: rotate(360deg); }
}
@media (max-width: 1100px) {
    .library-sound-card { padding-inline: 19px; }
}
@media (max-width: 700px) {
    .library-sound-card { padding: 21px 21px 16px; }
    .library-card-description { min-height: 0; font-size: 12px; }
    .library-card-category { font-size: 11px; }
    .library-card-actions button, .library-recipe { min-height: 42px; font-size: 11px; }
    .library-card-meta { font-size: 9px; }
    .library-variant-row select { min-height: 40px; }
}
@media (prefers-reduced-motion: reduce) {
    .library-spinner { animation: none; }
}
</style>
