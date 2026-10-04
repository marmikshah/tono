<script setup lang="ts">
import { computed } from "vue";
import type { AudioPlayer } from "../composables/useAudio";
import { sampleId } from "../data/catalog";
import type { LibrarySound } from "../data/library";
import Icon from "./Icon.vue";
import Waveform from "./Waveform.vue";

const props = withDefaults(defineProps<{
    sound: LibrarySound;
    player: AudioPlayer;
    variant: number;
    saved: boolean;
    compact: boolean;
    selected?: boolean;
    inlineDetails?: boolean;
}>(), { selected: false, inlineDetails: false });
const emit = defineEmits<{ select: []; audition: [] }>();
const sample = computed(() => props.sound.variants[props.variant] || props.sound.variants[0]);
const id = computed(() => sampleId(sample.value));
const active = computed(() => props.player.state.activeId === id.value);
const playing = computed(() => active.value && props.player.state.playing);
const loading = computed(() => props.player.state.loadingId === id.value);
const downloading = computed(() => props.player.state.downloadId === id.value);
const label = computed(() => sample.value.label || `Variation ${props.variant + 1}`);
const category = computed(() => props.sound.category === "ui" ? "UI" : props.sound.category.charAt(0).toUpperCase() + props.sound.category.slice(1));
const duration = computed(() => sample.value.duration < 1 ? `${sample.value.duration.toFixed(2)}s` : `${sample.value.duration.toFixed(1)}s`);
function auditionSound() {
    if (!playing.value && !loading.value) emit("audition");
    void props.player.toggleSample(sample.value);
}
function selectFromRow(event: MouseEvent) {
    // Small screens open details only through the explicit title action.
    if (props.inlineDetails || !(event.target instanceof Element) || event.target.closest("button, a, input, select")) return;
    emit("select");
}
</script>

<template>
    <article class="library-sound-card" :class="{ 'is-playing': playing, 'is-rendering': loading, 'is-compact': compact, 'is-music': sound.bpm, 'is-selected': selected }" :aria-labelledby="`library-${sound.id}-title`" @click="selectFromRow">
        <button class="library-play" type="button" :aria-busy="loading" :title="`${playing ? 'Pause' : 'Play'} ${sound.title}`" :aria-label="`${loading ? 'Cancel rendering' : playing ? 'Pause' : 'Play'} ${sound.title}, ${label}`" @click.stop="auditionSound"><span v-if="loading" class="library-spinner" aria-hidden="true" /><Icon v-else :name="playing ? 'pause' : 'play'" :size="17" /></button>
        <h3 class="library-card-identity">
            <button :id="`library-${sound.id}-select`" class="library-select" type="button" :aria-label="`Details for ${sound.title}`" :aria-pressed="selected" :aria-expanded="inlineDetails ? selected : undefined" :aria-controls="`sound-inspector-${sound.id}`" @click.stop="emit('select')">
                <span :id="`library-${sound.id}-title`" class="library-card-name"><span>{{ sound.title }}</span><Icon v-if="saved" name="star-filled" :size="12" class="library-saved-mark" /><Icon v-if="inlineDetails" name="chevron" :size="14" class="library-disclosure-chevron" /></span>
                <span class="library-card-meta"><span v-if="sound.bpm" class="library-score-meta">{{ sound.bpm }} BPM · {{ sound.key }}</span><template v-else><span>{{ category }}</span><span class="library-meta-separator">·</span><span>{{ label }}</span><span v-if="sound.looping" class="library-loop">Loop</span></template></span>
            </button>
        </h3>
        <div class="library-card-wave" aria-hidden="true"><Waveform continuous :values="sample.waveform" :progress="active ? player.state.progress : 0" /></div>
        <span class="library-card-duration">{{ duration }}</span>
        <button class="library-wav" type="button" :disabled="Boolean(player.state.downloadId)" :aria-busy="downloading" title="Download WAV" :aria-label="`Download ${sound.title}, ${label}, as WAV`" @click.stop="player.downloadSample(sample)"><span v-if="downloading" class="library-spinner" aria-hidden="true" /><Icon v-else name="download" :size="17" /></button>
    </article>
</template>

<style scoped>
.library-sound-card { position: relative; display: grid; grid-template-columns: 44px minmax(0, 1fr) 44px; grid-template-areas: "wave wave wave" "play identity download" "duration duration duration"; align-items: center; min-width: 0; gap: 8px; padding: 12px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface); }
.library-sound-card:hover { background: var(--surface-hover); }
.library-sound-card.is-selected { border-color: var(--accent); background: var(--surface-raised); }
.library-card-identity { grid-area: identity; min-width: 0; margin: 0; }
.library-select { display: flex; flex-direction: column; justify-content: center; width: 100%; min-width: 0; min-height: 44px; padding: 0; text-align: left; }
.library-card-name { display: flex; align-items: center; gap: 6px; overflow: hidden; width: 100%; color: var(--ink); font-size: 14px; font-weight: 500; line-height: 19px; white-space: nowrap; text-overflow: ellipsis; }
.library-card-name > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
.library-card-name > svg { flex-shrink: 0; }
.library-saved-mark { color: var(--accent-ink); }
.library-disclosure-chevron { margin-left: auto; color: var(--muted); }
.library-select[aria-expanded="true"] .library-disclosure-chevron { color: var(--accent-ink); transform: rotate(180deg); }
.library-card-meta { display: flex; align-items: center; gap: 5px; overflow: hidden; width: 100%; margin-top: 2px; color: var(--muted); font-size: 12px; font-weight: 400; line-height: 16px; white-space: nowrap; }
.library-card-meta > span { overflow: hidden; text-overflow: ellipsis; }
.library-card-meta > .library-meta-separator { flex-shrink: 0; }
.library-loop { color: var(--muted-strong); }
.library-card-wave { grid-area: wave; min-width: 0; padding: 24px 8px; border-radius: 5px; color: var(--muted); background: var(--paper); }
.library-card-wave :deep(.waveform) { width: 100%; height: 80px; }
.library-card-wave :deep(.wave-shape) { fill: var(--muted); opacity: .65; }
.library-card-wave :deep(.wave-played) { fill: var(--accent); opacity: 1; }
.library-card-wave :deep(.wave-baseline) { stroke: var(--line); }
.library-card-duration { grid-area: duration; color: var(--muted); font-size: 12px; font-variant-numeric: tabular-nums; }
.library-play, .library-wav { display: inline-flex; align-items: center; justify-content: center; width: 44px; height: 44px; padding: 0; border: 0; border-radius: 5px; color: var(--muted-strong); background: transparent; }
.library-play { grid-area: play; }
.library-wav { grid-area: download; }
.library-play:hover, .library-wav:hover { color: var(--ink); background: var(--surface-hover); }
.is-playing .library-play, .is-rendering .library-play { color: var(--on-accent); background: var(--accent); }
.library-wav:disabled { opacity: .55; cursor: wait; }
.is-compact { grid-template-columns: var(--library-columns); grid-template-areas: var(--library-areas); gap: var(--library-gap); height: 56px; padding: 5px 8px; border: 0; border-bottom: 1px solid color-mix(in srgb, var(--line) 55%, transparent); border-radius: 0; background: transparent; }
.is-compact:hover { background: var(--surface-hover); }
.is-compact.is-selected { background: var(--surface-raised); }
.is-compact.is-selected::before { position: absolute; inset: 0 auto 0 0; width: 2px; background: var(--accent); content: ''; }
.is-compact .library-card-wave { padding: 0; background: transparent; }
.is-compact .library-card-wave :deep(.waveform) { height: 30px; }
.is-compact .library-card-duration { justify-self: end; }
button:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
.library-spinner { width: 14px; height: 14px; border: 2px solid currentColor; border-right-color: transparent; border-radius: 50%; animation: library-spin .8s linear infinite; }
@keyframes library-spin { to { transform: rotate(360deg); } }
@media (max-width: 700px) { .is-compact { height: 64px; padding-block: 9px; } .is-compact .library-card-wave { display: none; } .library-card-name { font-size: 14px; } }
@media (min-width: 1200px) { .library-disclosure-chevron { display: none; } }
@media (prefers-reduced-motion: reduce) { .library-spinner { animation-duration: 1.5s; } }
</style>
