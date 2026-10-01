<script setup lang="ts">
import { computed } from "vue";
import type { AudioPlayer } from "../composables/useAudio";
import { musicUrl, scoreUrl, type MusicTrack } from "../data/catalog";
import CoverArt from "./CoverArt.vue";
import Icon from "./Icon.vue";

const props = defineProps<{ track: MusicTrack; player: AudioPlayer }>();
const active = computed(
    () => props.player.state.activeId === `music:${props.track.id}`,
);
const playing = computed(() => active.value && props.player.state.playing);
</script>

<template>
    <article class="music-card" :class="{ 'is-playing': playing }">
        <div class="music-art">
            <CoverArt :kind="track.art" /><span class="music-mood">{{
                track.mood
            }}</span
            ><button
                class="music-play"
                type="button"
                :aria-label="`${playing ? 'Pause' : 'Play'} ${track.title}`"
                @click="player.toggle(`music:${track.id}`, musicUrl(track))"
            >
                <Icon :name="playing ? 'pause' : 'play'" :size="19" />
            </button>
            <div
                class="music-progress"
                :style="{
                    width: `${active ? player.state.progress * 100 : 0}%`,
                }"
            />
        </div>
        <div class="music-card-body">
            <div class="music-title-row">
                <h3>{{ track.title }}</h3>
                <a
                    :href="scoreUrl(track)"
                    :target="track.source ? '_blank' : undefined"
                    :rel="track.source ? 'noopener noreferrer' : undefined"
                    :aria-label="
                        track.source
                            ? `View ${track.title} source code`
                            : `Learn to compose songs like ${track.title}`
                    "
                    ><Icon name="external" :size="17"
                /></a>
            </div>
            <p>{{ track.description }}</p>
            <span class="music-detail">{{ track.detail }}</span>
            <p
                v-if="active && player.state.error"
                class="audio-error"
                role="alert"
            >
                {{ player.state.error }}
            </p>
        </div>
    </article>
</template>
