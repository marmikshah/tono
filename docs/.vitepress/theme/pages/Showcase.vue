<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { withBase } from "vitepress";
import { useAudioPlayer } from "../composables/useAudio";
import { tracks } from "../data/catalog";
import MusicCard from "../components/MusicCard.vue";
import Icon from "../components/Icon.vue";

const player = useAudioPlayer();
const categories = ["Everything", "Game music", "Full pieces", "Sound studies"];
const selected = ref(categories[0]);
watch(selected, () => player.stop());
const filtered = computed(() =>
    selected.value === categories[0]
        ? tracks
        : tracks.filter((track) => track.category === selected.value),
);
</script>

<template>
    <main id="main-content" class="site-container listening-room" tabindex="-1">
        <a class="back-link" :href="withBase('/')"
            ><Icon name="arrow" :size="16" /> Back to tono</a
        >
        <div class="library-intro">
            <p class="eyebrow">THE LISTENING ROOM</p>
            <h1>A little code.<br /><em>A lot of feeling.</em></h1>
            <p>
                Game worlds, late-night grooves, familiar melodies. Every sound
                here is an engine render from a score or a sound graph. Press
                play and see what’s possible.
            </p>
        </div>
        <div class="filter-row">
            <div
                class="filter-tabs"
                role="group"
                aria-label="Filter the listening room"
            >
                <button
                    v-for="category in categories"
                    :key="category"
                    :aria-pressed="selected === category"
                    type="button"
                    @click="selected = category"
                >
                    {{ category }}
                </button>
            </div>
            <span class="result-count" aria-live="polite"
                >{{ filtered.length }} examples</span
            >
        </div>
        <div class="music-grid library-grid">
            <MusicCard
                v-for="track in filtered"
                :key="track.id"
                :track="track"
                :player="player"
            />
        </div>
        <div class="library-outro">
            <div>
                <h2>Have a melody in mind?</h2>
                <p>Give it a voice. Build your own score with the song API.</p>
            </div>
            <a class="button button-primary" :href="withBase('/guides/songs')"
                >Compose your first song <Icon name="arrow" :size="17"
            /></a>
        </div>
    </main>
</template>
