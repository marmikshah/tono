<script setup lang="ts">
import { computed, ref } from "vue";
import { withBase } from "vitepress";
import { useAudioPlayer } from "../composables/useAudio";
import { sounds, tracks } from "../data/catalog";
import Icon from "../components/Icon.vue";
import SoundPreview from "../components/SoundPreview.vue";
import SoundCard from "../components/SoundCard.vue";
import MusicCard from "../components/MusicCard.vue";
import CodeExample from "../components/CodeExample.vue";

const player = useAudioPlayer();
const categories = ["All sounds", "Arcade", "Action", "Movement", "Interface"];
const selected = ref(categories[0]);
const filtered = computed(() =>
    selected.value === categories[0]
        ? sounds
        : sounds.filter((sound) => sound.category === selected.value),
);
</script>

<template>
    <main id="main-content" tabindex="-1">
        <section class="site-container hero">
            <div class="hero-copy">
                <p class="eyebrow">
                    <span class="eyebrow-dot" /> FOR THE GAMES YOU’RE MAKING
                </p>
                <h1>Give your game<br />its own <em>sound.</em></h1>
                <p class="hero-description">
                    From the smallest coin pickup to the melody that stays with
                    you. Create sound effects and compose music with a little
                    code and a lot of possibility.
                </p>
                <div class="hero-actions">
                    <a class="button button-primary" href="#sounds"
                        >Find your sound <Icon name="arrow" :size="18" /></a
                    ><a class="text-link" :href="withBase('/get-started/')"
                        >Meet the engine <Icon name="arrow" :size="16"
                    /></a>
                </div>
                <p class="hero-note">
                    <span>Open source</span><i />Built in Rust<i />Made for play
                </p>
            </div>
            <SoundPreview :player="player" />
        </section>

        <div class="site-container capabilities">
            <div>
                <Icon name="wave" :size="24" />
                <p>
                    <strong>Small sounds. Big personality.</strong
                    ><span>8 ready-to-use sound effect starters</span>
                </p>
            </div>
            <div>
                <Icon name="layers" :size="23" />
                <p>
                    <strong>A whole band in your code.</strong
                    ><span>31 instrument voices to compose with</span>
                </p>
            </div>
            <div>
                <Icon name="code" :size="23" />
                <p>
                    <strong>Make it. Shape it. Ship it.</strong
                    ><span>Editable recipes. WAV, FLAC &amp; OGG exports.</span>
                </p>
            </div>
        </div>

        <section id="sounds" class="site-container sounds-section">
            <div class="section-heading">
                <div>
                    <p class="eyebrow">A GOOD PLACE TO START</p>
                    <h2>Little sounds. <em>Instant character.</em></h2>
                    <p>Press play. Find a favorite. Make it your own.</p>
                </div>
                <a class="text-link" :href="withBase('/guides/generation')"
                    >Explore the generator <Icon name="arrow" :size="17"
                /></a>
            </div>
            <div class="filter-row">
                <div
                    class="filter-tabs"
                    role="group"
                    aria-label="Filter sound effects"
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
                    >{{ filtered.length }} sounds</span
                >
            </div>
            <div class="sound-grid">
                <SoundCard
                    v-for="sound in filtered"
                    :key="sound.id"
                    :sound="sound"
                    :player="player"
                />
            </div>
            <p class="collection-note">
                <Icon name="wave" :size="14" /> Real engine renders. Try four
                seeded variations of each sound in the Sound Lab above.
            </p>
        </section>

        <section id="music" class="music-section">
            <div class="site-container">
                <div class="section-heading">
                    <div>
                        <p class="eyebrow">SET THE SCENE</p>
                        <h2>A world deserves <em>a soundtrack.</em></h2>
                        <p>
                            A few things made with tono. A little inspiration
                            for yours.
                        </p>
                    </div>
                    <a class="text-link" :href="withBase('/showcase')"
                        >Visit the listening room <Icon name="arrow" :size="17"
                    /></a>
                </div>
                <div class="music-grid">
                    <MusicCard
                        v-for="track in tracks.slice(0, 3)"
                        :key="track.id"
                        :track="track"
                        :player="player"
                    />
                </div>
                <p class="collection-note">
                    Every track is rendered by tono. Open a score to see how
                    it’s made.
                </p>
            </div>
        </section>

        <section class="site-container workflow-section">
            <div class="workflow-copy">
                <p class="eyebrow">CREATIVE CONTROL, DOWN TO THE NOTE</p>
                <h2>Your sound.<br /><em>Your rules.</em></h2>
                <p>
                    Start with a recipe, then follow your ear. Change the seed,
                    shape the sound, or build a song from scratch. The source
                    stays editable from first idea to final export.
                </p>
                <ul class="workflow-points">
                    <li>
                        <Icon name="check" :size="17" /> Repeatable sounds from
                        a recipe and a seed
                    </li>
                    <li>
                        <Icon name="check" :size="17" /> Rust for sound effects.
                        Python for composition.
                    </li>
                    <li>
                        <Icon name="check" :size="17" /> Audio files ready for
                        your game engine
                    </li>
                </ul>
                <a class="text-link" :href="withBase('/get-started/quickstart')"
                    >Your first sound, in a few minutes
                    <Icon name="arrow" :size="16"
                /></a>
            </div>
            <CodeExample />
        </section>

        <section class="closing-section site-container">
            <div class="closing-wave" aria-hidden="true">
                <i
                    v-for="(height, index) in [
                        10, 22, 36, 52, 68, 84, 68, 52, 36, 22, 10,
                    ]"
                    :key="index"
                    :style="{ height: `${height}px` }"
                />
            </div>
            <h2>What will your game <em>sound like?</em></h2>
            <p>Start small. Make something you want to press play on.</p>
            <a
                class="button button-primary"
                :href="withBase('/guides/generation')"
                >Start creating <Icon name="arrow" :size="18"
            /></a>
        </section>
    </main>
</template>
