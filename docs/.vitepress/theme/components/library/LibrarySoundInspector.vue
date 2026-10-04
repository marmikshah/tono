<script setup lang="ts">
import { computed } from "vue";
import type { AudioPlayer } from "../../composables/useAudio";
import { sampleId, sampleUrl } from "../../data/catalog";
import type { LibrarySound } from "../../data/library";
import Icon from "../Icon.vue";
import Waveform from "../Waveform.vue";

const props = withDefaults(defineProps<{
    sound: LibrarySound;
    player: AudioPlayer;
    variant: number;
    saved: boolean;
    inline?: boolean;
}>(), { inline: false });
const emit = defineEmits<{ variant: [index: number]; save: []; close: [] }>();
const sample = computed(() => props.sound.variants[props.variant] || props.sound.variants[0]);
const active = computed(() => props.player.state.activeId === sampleId(sample.value));
const playing = computed(() => active.value && props.player.state.playing);
const loading = computed(() => props.player.state.loadingId === sampleId(sample.value));
const downloading = computed(() => props.player.state.downloadId === sampleId(sample.value));
const category = computed(() => props.sound.category === "ui" ? "UI" : props.sound.category.charAt(0).toUpperCase() + props.sound.category.slice(1));
const label = computed(() => sample.value.label || `Variation ${props.variant + 1}`);
function closeOnEscape(event: KeyboardEvent) {
    if (!props.inline) return;
    event.preventDefault();
    event.stopPropagation();
    emit("close");
}
</script>

<template>
    <section :id="`sound-inspector-${sound.id}`" class="library-sound-inspector" :class="{ 'is-inline': inline }" :aria-label="`Sound details: ${sound.title}`" @keydown.esc="closeOnEscape">
        <header class="inspector-header">
            <span>Sound details</span>
            <button v-if="inline" type="button" class="inspector-close" :aria-label="`Close details for ${sound.title}`" @click="emit('close')"><Icon name="close" :size="18" /></button>
        </header>
        <div class="inspector-identity">
            <p class="inspector-category">{{ category }}<span v-if="sound.looping">Seamless loop</span></p>
            <h3>{{ sound.title }}</h3>
            <p class="inspector-description">{{ sound.description }}</p>
        </div>
        <div class="inspector-wave" aria-hidden="true">
            <Waveform continuous :values="sample.waveform" :progress="active ? player.state.progress : 0" />
            <div><span>{{ label }}</span><span>{{ sample.duration.toFixed(2) }}s</span></div>
        </div>
        <div class="inspector-variant">
            <label :for="`inspector-${sound.id}-variant`">{{ sound.bpm ? 'Arrangement' : 'Variation' }}</label>
            <select :id="`inspector-${sound.id}-variant`" :value="variant" :aria-label="`${sound.title} ${sound.bpm ? 'arrangement' : 'variation'}`" @change="emit('variant', Number(($event.target as HTMLSelectElement).value))"><option v-for="(take, index) in sound.variants" :key="sampleId(take)" :value="index">{{ take.label || `Variation ${index + 1}` }}</option></select>
        </div>
        <dl v-if="sound.bpm" class="inspector-music-meta"><div><dt>Tempo</dt><dd>{{ sound.bpm }} BPM</dd></div><div><dt>Key</dt><dd>{{ sound.key }}</dd></div><div><dt>Loop</dt><dd>{{ sound.bars }} bars</dd></div></dl>
        <div class="inspector-audio-actions">
            <button class="inspector-audition" type="button" :aria-busy="loading" :aria-label="`${loading ? 'Cancel' : playing ? 'Pause' : 'Play'} selected sound: ${sound.title}, ${label}`" @click="player.toggleSample(sample)"><span v-if="loading" class="inspector-spinner" aria-hidden="true" /><Icon v-else :name="playing ? 'pause' : 'play'" :size="16" />{{ loading ? 'Cancel' : playing ? 'Pause' : 'Audition' }}</button>
            <button class="inspector-wav" type="button" :disabled="Boolean(player.state.downloadId)" :aria-busy="downloading" :aria-label="`Download selected ${sound.title}, ${label}, as WAV`" @click="player.downloadSample(sample)"><span v-if="downloading" class="inspector-spinner" aria-hidden="true" /><Icon v-else name="download" :size="16" />{{ downloading ? 'Preparing…' : 'WAV' }}</button>
        </div>
        <div class="inspector-source">
            <h4>Editable source</h4>
            <a class="library-recipe" :href="sampleUrl(sample.source)" :download="sample.source.split('/').pop()" :aria-label="`Download the editable recipe for ${sound.title}, ${label}`"><Icon name="code" :size="16" />Download recipe<Icon name="download" :size="14" /></a>
            <a v-if="sample.score" class="library-score" :href="sampleUrl(sample.score)" :download="sample.score.split('/').pop()" :aria-label="`Download the editable song score for ${sound.title}, ${label}`"><Icon name="layers" :size="16" />Download song score<Icon name="download" :size="14" /></a>
        </div>
        <button class="inspector-save" type="button" :aria-pressed="saved" :aria-label="`${saved ? 'Remove' : 'Save'} ${sound.title}${saved ? ' from saved sounds' : ''}`" @click="emit('save')"><Icon :name="saved ? 'star-filled' : 'star'" :size="16" />{{ saved ? 'Saved on this device' : 'Save on this device' }}</button>
        <details class="inspector-render-details"><summary>Render details<Icon name="chevron" :size="15" /></summary><dl><div><dt>Format</dt><dd>{{ sample.sampleRate / 1000 }} kHz stereo</dd></div><div><dt>Seed</dt><dd>{{ sample.seed }}</dd></div></dl></details>
        <div class="inspector-tags"><span v-for="tag in sound.tags" :key="tag">{{ tag }}</span></div>
        <p class="inspector-license">MIT licensed. Free to use.</p>
    </section>
</template>

<style scoped>
.library-sound-inspector { min-width: 0; padding: 16px; color: var(--ink); background: var(--surface); border: 1px solid var(--line); border-radius: 8px; }
.inspector-header { display: flex; align-items: center; justify-content: space-between; min-height: 24px; margin-bottom: 8px; color: var(--muted); font-size: 12px; }
.inspector-close { display: inline-flex; align-items: center; justify-content: center; width: 44px; height: 44px; color: var(--muted-strong); }
.inspector-close:hover { color: var(--ink); background: var(--surface-hover); }
.inspector-category { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin-bottom: 6px; color: var(--muted); font-size: 12px; }
.inspector-category span { color: var(--accent-ink); }
.inspector-identity h3 { color: var(--ink); font-size: 20px; font-weight: 600; letter-spacing: -.5px; line-height: 1.3; }
.inspector-description { margin-top: 8px; color: var(--muted-strong); font-size: 14px; line-height: 1.55; }
.inspector-wave { margin-block: 12px; padding: 14px 12px 10px; border-radius: 5px; color: var(--muted); background: var(--paper); }
.inspector-wave :deep(.waveform) { width: 100%; height: 56px; }
.inspector-wave :deep(.wave-shape) { fill: var(--muted); opacity: .7; }
.inspector-wave :deep(.wave-played) { fill: var(--accent); opacity: 1; }
.inspector-wave :deep(.wave-baseline) { stroke: var(--line); }
.inspector-wave > div { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-top: 8px; color: var(--muted); font-size: 12px; font-variant-numeric: tabular-nums; }
.inspector-variant { margin-bottom: 12px; }
.inspector-variant label { display: block; margin-bottom: 6px; color: var(--muted); font-size: 12px; }
.inspector-variant select { width: 100%; min-height: 44px; padding: 10px 30px 10px 12px; border: 1px solid var(--line); border-radius: 5px; color: var(--ink); background: var(--surface-raised); font-family: inherit; font-size: 13px; cursor: pointer; }
.inspector-music-meta { display: grid; grid-template-columns: 1fr 1.2fr .8fr; gap: 8px; margin: 0 0 16px; }
dt { color: var(--muted); font-size: 12px; }
dd { margin: 3px 0 0; color: var(--ink); font-size: 13px; font-variant-numeric: tabular-nums; }
.inspector-audio-actions { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
.inspector-audio-actions button { display: inline-flex; align-items: center; justify-content: center; gap: 8px; min-height: 44px; border-radius: 5px; font-size: 13px; font-weight: 500; }
.inspector-audition { color: var(--ink); background: var(--surface-raised); }
.inspector-audition:hover { background: var(--surface-hover); }
.inspector-wav { color: var(--on-accent); background: var(--accent); }
.inspector-wav:hover { background: var(--accent-hover); }
.inspector-wav:disabled { opacity: .55; cursor: wait; }
.inspector-save { display: inline-flex; align-items: center; justify-content: center; gap: 8px; width: 100%; min-height: 44px; margin-top: 8px; color: var(--muted-strong); font-size: 13px; }
.inspector-save:hover, .inspector-save[aria-pressed="true"] { color: var(--accent-ink); }
.inspector-source { margin-top: 16px; padding-top: 14px; border-top: 1px solid var(--line); }
.inspector-source h4 { margin: 0 0 5px; color: var(--muted); font-size: 12px; font-weight: 400; }
.inspector-source a { display: flex; align-items: center; gap: 8px; min-height: 44px; color: var(--muted-strong); font-size: 13px; }
.inspector-source a > svg:last-child { margin-left: auto; color: var(--muted); }
.inspector-source a:hover { color: var(--accent-ink); }
.inspector-render-details { margin-top: 10px; }
.inspector-render-details summary { display: flex; align-items: center; justify-content: space-between; min-height: 44px; color: var(--muted); font-size: 12px; cursor: pointer; list-style: none; }
.inspector-render-details summary::-webkit-details-marker { display: none; }
.inspector-render-details[open] summary svg { transform: rotate(180deg); }
.inspector-render-details dl { margin: 0; }
.inspector-render-details dl > div { display: flex; justify-content: space-between; gap: 16px; padding-bottom: 8px; }
.inspector-render-details dd { margin: 0; }
.inspector-tags { display: flex; flex-wrap: wrap; gap: 5px; margin-top: 10px; }
.inspector-tags span { padding: 3px 6px; border-radius: 3px; color: var(--muted); background: var(--surface-raised); font-size: 12px; }
.inspector-license { margin-top: 14px; color: var(--muted); font-size: 12px; }
button:focus-visible, select:focus-visible, a:focus-visible, summary:focus-visible { outline: 2px solid var(--focus); outline-offset: 3px; }
.inspector-spinner { width: 14px; height: 14px; border: 2px solid currentColor; border-right-color: transparent; border-radius: 50%; animation: inspector-spin .8s linear infinite; }
@keyframes inspector-spin { to { transform: rotate(360deg); } }
.is-inline { margin: 8px 0 16px; }
.is-inline .inspector-header { margin-bottom: 8px; }
@media (min-width: 700px) { .is-inline { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 0 24px; } .is-inline .inspector-header, .is-inline .inspector-identity { grid-column: 1; } .is-inline .inspector-header { grid-column: 1 / -1; } .is-inline .inspector-wave { grid-column: 2; grid-row: 2 / span 2; margin-top: 0; align-self: start; } .is-inline .inspector-variant { margin-top: 16px; } .is-inline .inspector-audio-actions { align-self: start; } .is-inline .inspector-source { grid-column: 2; grid-row: 4 / span 3; margin-top: 0; align-self: start; } .is-inline .inspector-tags, .is-inline .inspector-license { grid-column: 1 / -1; } }
@media (prefers-reduced-motion: reduce) { .inspector-spinner { animation-duration: 1.5s; } }
</style>
