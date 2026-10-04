<script setup lang="ts">
import { computed, ref, watch } from "vue";
import Icon from "./Icon.vue";
import { useAudioPlayer } from "../composables/useAudio";

const octave = ref(4);
const tempo = ref(120);
const instrument = ref("piano");
const hold = ref(1);
const player = useAudioPlayer();
const beatSeconds = computed(() => 60 / tempo.value);
const notes = computed(() => [`C${octave.value}`, `E${octave.value}`, `G${octave.value}`, `C${octave.value + 1}`]);
const firstHz = computed(() => 440 * 2 ** (((octave.value + 1) * 12 - 69) / 12));
const playing = computed(() => player.state.playing);
const loading = computed(() => Boolean(player.state.loadingId));
const currentBeat = computed(() => {
    const beat = player.state.currentTime / beatSeconds.value;
    return playing.value && beat % 1 < hold.value ? Math.floor(beat) : -1;
});
const playLabel = computed(() => loading.value ? "Cancel" : playing.value ? "Pause" : player.state.progress > 0 && player.state.progress < 1 ? "Resume" : "Play phrase");
const recipe = computed(() => ({
    name: "learn-the-basics", version: 2, engine: 5, seed: 42,
    sample_rate: 48_000, duration: 4 * beatSeconds.value,
    root: { type: "chain", stages: [
        { type: "seq", bpm: tempo.value, steps_per_beat: 4, wave: instrument.value,
          env: { a: 0.005, d: 0.05, s: 0.5, r: 0.05 },
          notes: notes.value.map((pitch, index) => ({ step: index * 4, len: hold.value * 4, pitch, gain: 0.7 })) },
        { type: "gain", amount: 0.35 },
    ] },
}));
watch([octave, tempo, instrument, hold], () => player.stop());
function play() { void player.toggleDocument("learn-the-basics", recipe.value); }
</script>

<template>
    <section class="learn-sound" aria-label="Explore pitch, tempo and instruments">
        <div class="learn-sound-heading"><strong>One phrase. Four things to change.</strong><span>Change a setting, then press play.</span></div>
        <div class="learn-controls">
            <label>Pitch <select v-model.number="octave"><option :value="3">C3 · low</option><option :value="4">C4 · middle</option><option :value="5">C5 · high</option></select><small>First note: {{ firstHz.toFixed(1) }} Hz</small></label>
            <label>Instrument <select v-model="instrument"><option value="piano">Piano</option><option value="pluck">Pluck</option><option value="sine">Sine tone</option></select><small>The character of each note</small></label>
            <label>Tempo <span class="learn-range"><input v-model.number="tempo" type="range" min="60" max="180" step="10" aria-label="Tempo in BPM" /><output>{{ tempo }} BPM</output></span><small>One beat: {{ beatSeconds.toFixed(2) }}s</small></label>
            <label>Note length <select v-model.number="hold"><option :value="0.5">Half a beat</option><option :value="1">One beat</option></select><small>How long each note lasts</small></label>
        </div>
        <ol class="learn-sequence" aria-label="One bar with four beats">
            <li v-for="(note, index) in notes" :key="index" :class="{ sounding: currentBeat === index }"><span>Beat {{ index + 1 }}</span><strong>{{ note }}</strong><i :style="{ width: `${hold * 100}%` }" aria-hidden="true" /></li>
        </ol>
        <p class="learn-timing">One bar = 4 beats = <strong>{{ (4 * beatSeconds).toFixed(2) }} seconds</strong>. Shorter notes leave space before the next beat.</p>
        <div class="learn-transport"><button class="learn-play" type="button" :aria-busy="loading" @click="play"><Icon :name="loading ? 'close' : playing ? 'pause' : 'play'" :size="16" />{{ playLabel }}</button><button class="learn-stop" type="button" :disabled="!loading && !player.state.playing && !player.state.progress" @click="player.stop()">Stop</button><span>Plays one bar.</span></div>
        <p v-if="player.state.error" class="learn-error" role="alert">{{ player.state.error }}</p>
    </section>
</template>

<style scoped>
.learn-sound { margin-block: 24px; padding: 20px; border: 1px solid var(--line); border-radius: 10px; background: var(--surface); }
.learn-sound-heading { display: flex; flex-direction: column; gap: 4px; margin-bottom: 20px; }
.learn-sound-heading strong { color: var(--ink); font-size: 15px; }
.learn-sound-heading > span { color: var(--muted); font-size: 13px; }
.learn-controls { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 18px; }
.learn-controls label { display: flex; flex-direction: column; gap: 7px; min-width: 0; color: var(--ink); font-size: 13px; font-weight: 500; }
.learn-controls select { width: 100%; min-height: 44px; padding: 8px 10px; border: 1px solid var(--line); border-radius: 5px; background: var(--paper); color: var(--ink); font: inherit; }
.learn-controls small { color: var(--muted); font-size: 12px; font-weight: 400; line-height: 1.4; }
.learn-range { display: flex; align-items: center; gap: 12px; min-height: 44px; }
.learn-range input { width: 100%; min-width: 0; height: 44px; accent-color: var(--accent); }
.learn-range output { flex-shrink: 0; min-width: 65px; color: var(--accent-ink); font-size: 13px; }
.learn-sequence { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 6px; padding: 0 !important; margin: 22px 0 0 !important; list-style: none !important; }
.learn-sequence li { position: relative; display: flex; flex-direction: column; gap: 6px; min-width: 0; margin: 0 !important; padding: 10px 8px 14px; border: 1px solid var(--line); border-radius: 5px; background: var(--paper); }
.learn-sequence li > span { color: var(--muted); font-size: 11px; white-space: nowrap; }
.learn-sequence strong { color: var(--ink); font-size: 15px; font-weight: 500; }
.learn-sequence i { position: absolute; bottom: 0; left: 0; height: 3px; background: var(--accent); border-radius: 2px; }
.learn-sequence .sounding { border-color: var(--accent); background: var(--accent-soft); }
.learn-timing { margin: 12px 0 0 !important; color: var(--muted-strong); font-size: 13px; }
.learn-timing strong { color: var(--ink); }
.learn-transport { display: flex; align-items: center; flex-wrap: wrap; gap: 10px; margin-top: 18px; }
.learn-transport button { min-height: 44px; padding: 9px 14px; border-radius: 5px; font: inherit; font-size: 13px; font-weight: 500; cursor: pointer; }
.learn-play { display: inline-flex; align-items: center; gap: 8px; background: var(--accent); color: var(--on-accent); }
.learn-stop { border: 1px solid var(--line); color: var(--ink); }
.learn-stop:disabled { opacity: .4; cursor: default; }
.learn-transport > span { margin-left: auto; color: var(--muted); font-size: 12px; }
.learn-sound :is(button, select, input):focus-visible { outline: 2px solid var(--focus); outline-offset: 3px; }
.learn-error { margin-bottom: 0 !important; color: var(--danger); font-size: 13px; }
@media (max-width: 480px) { .learn-sound { padding: 16px 12px; } .learn-controls { gap: 16px; } .learn-controls label:nth-child(n+3) { grid-column: 1 / -1; } .learn-transport > span { width: 100%; margin-left: 0; } }
</style>
