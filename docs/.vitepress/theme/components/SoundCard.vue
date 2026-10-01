<script setup lang="ts">
import { computed } from "vue";
import type { AudioPlayer } from "../composables/useAudio";
import { sampleId, sampleUrl, type Sound } from "../data/catalog";
import Icon from "./Icon.vue";
import Waveform from "./Waveform.vue";

const props = defineProps<{ sound: Sound; player: AudioPlayer }>();
const sample = computed(() => props.sound.takes[0]);
const active = computed(
    () => props.player.state.activeId === sampleId(sample.value),
);
const playing = computed(() => active.value && props.player.state.playing);
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
                @click="
                    player.toggle(sampleId(sample), sampleUrl(sample.audio))
                "
            >
                <Icon :name="playing ? 'pause' : 'play'" :size="15" />
            </button>
            <span>{{ sample.duration.toFixed(2) }}s <b>·</b> WAV</span>
            <a
                class="sound-download"
                :href="sampleUrl(sample.audio)"
                :download="sample.audio"
                :aria-label="`Download ${sound.title} as WAV`"
                ><Icon name="download" :size="17"
            /></a>
        </div>
        <p v-if="active && player.state.error" class="audio-error" role="alert">
            {{ player.state.error }}
        </p>
    </article>
</template>
