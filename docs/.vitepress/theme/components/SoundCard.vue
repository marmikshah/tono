<script setup lang="ts">
import { computed } from "vue";
import type { AudioPlayer } from "../composables/useAudio";
import { sampleId, type Sound } from "../data/catalog";
import Icon from "./Icon.vue";
import Waveform from "./Waveform.vue";

const props = defineProps<{ sound: Sound; player: AudioPlayer }>();
const sample = computed(() => props.sound.takes[0]);
const active = computed(
    () => props.player.state.activeId === sampleId(sample.value),
);
const playing = computed(() => active.value && props.player.state.playing);
const loading = computed(() => props.player.state.loadingId === sampleId(sample.value));
const downloading = computed(() => props.player.state.downloadId === sampleId(sample.value));
</script>

<template>
    <article
        class="sound-card"
        :class="[`sound-${sound.color}`, { 'is-playing': playing }]"
    >
        <div class="sound-card-top">
            <span class="sound-icon"><Icon :name="sound.id" :size="19" /></span
            ><span class="sound-category">{{ sound.category }}</span>
        </div>
        <h3>{{ sound.title }}</h3>
        <p>{{ sound.description }}</p>
        <Waveform
            compact
            :values="sample.waveform"
            :progress="active ? player.state.progress : 0"
        />
        <div class="sound-card-bottom">
            <button
                class="round-play"
                type="button"
                :aria-label="`${playing ? 'Pause' : 'Play'} ${sound.title}`"
                :aria-busy="loading"
                @click="player.toggleSample(sample)"
            >
                <Icon :name="playing ? 'pause' : 'play'" :size="15" />
            </button>
            <span>{{ sample.duration.toFixed(2) }}s <b>·</b> WAV</span>
            <button
                class="sound-download"
                type="button"
                :disabled="!!player.state.downloadId"
                :aria-busy="downloading"
                @click="player.downloadSample(sample)"
                :aria-label="`Download ${sound.title} as WAV`"
                ><Icon name="download" :size="17"
            /></button>
        </div>
        <p v-if="active && player.state.error" class="audio-error" role="alert">
            {{ player.state.error }}
        </p>
    </article>
</template>
