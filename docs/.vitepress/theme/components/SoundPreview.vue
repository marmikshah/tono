<script setup lang="ts">
import { computed, ref } from "vue";
import type { AudioPlayer } from "../composables/useAudio";
import { sounds, sampleId, sampleUrl } from "../data/catalog";
import Icon from "./Icon.vue";
import Waveform from "./Waveform.vue";

const props = defineProps<{ player: AudioPlayer }>();
const selected = ref(sounds[0].id);
const take = ref(0);
const sound = computed(
    () => sounds.find((sound) => sound.id === selected.value)!,
);
const sample = computed(() => sound.value.takes[take.value]);
const active = computed(
    () => props.player.state.activeId === sampleId(sample.value),
);
const playing = computed(() => active.value && props.player.state.playing);
const loading = computed(() => props.player.state.loadingId === sampleId(sample.value));
const downloading = computed(() => props.player.state.downloadId === sampleId(sample.value));

function select(id: string) {
    props.player.stop();
    selected.value = id;
    take.value = 0;
}
function next() {
    props.player.stop();
    take.value = (take.value + 1) % sound.value.takes.length;
}
</script>

<template>
    <section class="sound-preview" aria-label="Try the sound effects">
        <div class="preview-topline">
            <span><Icon name="wave" :size="16" /> TRY A SOUND</span
            ><span class="preview-status"><i /> {{ loading ? 'Preparing…' : playing ? 'Playing' : 'Ready to play' }}</span>
        </div>
        <div class="preset-grid" role="group" aria-label="Sound presets">
            <button
                v-for="preset in sounds"
                :key="preset.id"
                type="button"
                :aria-pressed="selected === preset.id"
                @click="select(preset.id)"
            >
                <Icon :name="preset.id" :size="15" />{{
                    preset.id === "coin" ? "Coin" : preset.title
                }}
            </button>
        </div>
        <div class="preview-title">
            <h2>{{ sound.title }}</h2>
            <span>{{ sound.category }}</span>
        </div>
        <div class="preview-scope">
            <Waveform
                :values="sample.waveform"
                :progress="active ? player.state.progress : 0"
            />
            <div class="scope-meta">
                <span aria-live="polite">VARIATION {{ take + 1 }} OF {{ sound.takes.length }}</span
                ><span>{{ sample.duration.toFixed(2) }} SEC</span>
            </div>
        </div>
        <div class="preview-controls">
            <button
                class="preview-play"
                type="button"
                :aria-label="`${loading ? 'Cancel rendering' : playing ? 'Pause' : 'Play'} ${sound.title} preview`"
                :aria-busy="loading"
                @click="player.toggleSample(sample)"
            >
                <Icon :name="playing ? 'pause' : 'play'" :size="17" />{{
                    loading ? "Cancel" : playing ? "Pause sound" : "Play sound"
                }}
            </button>
            <button
                class="preview-shuffle"
                type="button"
                aria-label="Next sound variation"
                title="Try the next variation"
                @click="next"
            >
                <Icon name="arrow" :size="18" />
            </button>
            <button
                class="preview-download"
                type="button"
                :disabled="!!player.state.downloadId"
                :aria-busy="downloading"
                @click="player.downloadSample(sample)"
                :aria-label="`Download ${sound.title} variation ${take + 1} as WAV`"
                ><Icon name="download" :size="16" /> {{ downloading ? 'Rendering…' : 'WAV' }}</button
            >
        </div>
        <details class="preview-details">
            <summary>Recipe &amp; audio details <Icon name="chevron" :size="13" /></summary>
            <div class="preview-bottom"><span>Seed {{ sample.seed }}</span><span>{{ sample.sampleRate / 1000 }} kHz / 16-bit stereo</span></div>
            <a
            class="preview-source"
            :href="sampleUrl(sample.source)"
            :download="sample.source"
            :aria-label="`Download the editable ${sound.title} recipe`"
            >Editable recipe <Icon name="download" :size="12"
        /></a>
        </details>
        <p class="audio-error" v-if="player.state.error" role="alert">
            {{ player.state.error }}
        </p>
    </section>
</template>

<style scoped>
.preview-details { margin-top: 16px; padding-top: 13px; border-top: 1px solid var(--line); }
.preview-details summary { display: flex; justify-content: space-between; align-items: center; min-height: 30px; color: var(--muted); font-size: 12px; list-style: none; }
.preview-details summary::-webkit-details-marker { display: none; }
.preview-details[open] summary svg { transform: rotate(180deg); }
.preview-details .preview-bottom { margin-top: 7px; font-size: 12px; }
.preview-details .preview-source { min-height: 36px; font-size: 12px; }
</style>
