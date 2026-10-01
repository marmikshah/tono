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
            <span><Icon name="wave" :size="16" /> THE SOUND LAB</span
            ><span class="preview-status"><i /> Engine preview</span>
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
                <span>AMPLITUDE</span
                ><span>{{ sample.duration.toFixed(2) }} SEC</span>
            </div>
        </div>
        <div class="preview-controls">
            <button
                class="preview-play"
                type="button"
                :aria-label="`${playing ? 'Pause' : 'Play'} ${sound.title} preview`"
                @click="
                    player.toggle(sampleId(sample), sampleUrl(sample.audio))
                "
            >
                <Icon :name="playing ? 'pause' : 'play'" :size="17" />{{
                    playing ? "Pause sound" : "Play sound"
                }}
            </button>
            <button
                class="preview-shuffle"
                type="button"
                aria-label="Next sound variation"
                title="Try the next variation"
                @click="next"
            >
                <Icon name="shuffle" :size="18" />
            </button>
            <a
                class="preview-download"
                :href="sampleUrl(sample.audio)"
                :download="sample.audio"
                :aria-label="`Download ${sound.title} variation ${take + 1} as WAV`"
                ><Icon name="download" :size="16" /> WAV</a
            >
        </div>
        <div class="preview-bottom">
            <span aria-live="polite"
                >Variation {{ take + 1 }} of {{ sound.takes.length }}
                <b>·</b> Seed {{ sample.seed }}</span
            ><span>{{ sample.sampleRate / 1000 }} kHz / 16-bit</span>
        </div>
        <a
            class="preview-source"
            :href="sampleUrl(sample.source)"
            :download="sample.source"
            :aria-label="`Download the editable ${sound.title} recipe`"
            >Editable recipe <Icon name="download" :size="12"
        /></a>
        <p class="audio-error" v-if="active && player.state.error" role="alert">
            {{ player.state.error }}
        </p>
    </section>
</template>
