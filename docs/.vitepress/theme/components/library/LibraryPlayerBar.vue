<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { AudioPlayer } from "../../composables/useAudio";
import Icon from "../Icon.vue";

const props = withDefaults(defineProps<{
    player: AudioPlayer;
    title: string;
    subtitle: string;
    onDownload: () => void | Promise<void>;
    downloadLabel?: string;
}>(), { downloadLabel: "Download WAV" });
const loading = computed(() => Boolean(props.player.state.loadingId));
const volumeOpen = ref(false);
const bar = ref<HTMLElement>();
let heightObserver: ResizeObserver | undefined;
let pageShell: HTMLElement | undefined;
const status = computed(() => loading.value ? "Preparing preview…" : props.player.state.playing ? "Now playing" : props.player.state.progress >= 1 ? "Finished" : "Paused");
const time = (seconds: number) => props.player.state.duration < 10
    ? `${Math.max(0, seconds).toFixed(1)}s`
    : `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, "0")}`;

function onKeydown(event: KeyboardEvent) {
    if ((event.code !== "Space" && event.key !== " ") || event.repeat || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || !props.player.state.activeId) return;
    const target = event.target;
    if (target instanceof Element && target.closest("input, textarea, select, button, a, summary, [contenteditable]:not([contenteditable='false']), [role='button']")) return;
    event.preventDefault();
    void props.player.toggleActive();
}
onMounted(() => window.addEventListener("keydown", onKeydown));
watch(bar, (element) => {
    heightObserver?.disconnect();
    pageShell?.style.removeProperty("--preview-player-height");
    pageShell = element?.closest<HTMLElement>(".tono-site") || undefined;
    if (!element || !pageShell) return;
    const measure = () => pageShell?.style.setProperty("--preview-player-height", `${Math.ceil(element.getBoundingClientRect().height)}px`);
    heightObserver = new ResizeObserver(measure);
    heightObserver.observe(element);
    measure();
}, { flush: "post" });
onBeforeUnmount(() => {
    window.removeEventListener("keydown", onKeydown);
    heightObserver?.disconnect();
    pageShell?.style.removeProperty("--preview-player-height");
});
</script>

<template>
    <section v-if="player.state.activeId" ref="bar" class="library-player-bar" aria-label="Preview player">
        <div class="library-player-inner">
            <div class="library-player-track">
                <span class="library-player-mark" aria-hidden="true"><Icon name="headphones" :size="21" /></span>
                <div class="library-player-copy">
                <p class="library-player-status">{{ status }} <span v-if="player.state.looping">· Loop</span></p>
                <strong>{{ title }}</strong>
                <p class="library-player-subtitle">{{ subtitle }}</p>
                </div>
            </div>
            <div class="library-player-controls">
                <button class="library-player-toggle" type="button" :aria-label="loading ? 'Cancel preview' : player.state.playing ? 'Pause preview' : 'Play preview'"
                    :aria-busy="loading" @click="player.toggleActive()">
                    <span v-if="loading" class="library-player-spinner" aria-hidden="true" /><Icon v-else :name="player.state.playing ? 'pause' : 'play'" :size="20" />
                </button>
                <button class="library-player-stop" type="button" aria-label="Stop preview" @click="player.stop()"><Icon name="stop" :size="17" /> Stop</button>
            </div>
            <div class="library-player-position">
                <input type="range" aria-label="Playback position" min="0" max="1" step="0.001" :value="player.state.progress"
                    :style="{ '--range-progress': `${player.state.progress * 100}%` }"
                    :aria-valuetext="`${time(player.state.currentTime)} of ${time(player.state.duration)}`"
                    :disabled="loading || !player.state.duration" @input="player.seek(Number(($event.target as HTMLInputElement).value))" />
                <div class="library-player-time"><span>{{ time(player.state.currentTime) }} / {{ time(player.state.duration) }}</span><span class="library-player-shortcut"><kbd>Space</kbd> play / pause</span></div>
            </div>
            <div class="library-player-volume">
                <Icon class="library-volume-icon" :name="player.state.volume ? 'volume' : 'volume-off'" :size="19" />
                <button class="library-volume-toggle" type="button" aria-label="Volume controls" :aria-expanded="volumeOpen" @click="volumeOpen = !volumeOpen"><Icon :name="player.state.volume ? 'volume' : 'volume-off'" :size="19" /></button>
                <div class="library-volume-panel" :class="{ 'is-open': volumeOpen }"><input type="range" aria-label="Preview volume" min="0" max="1" step="0.05" :value="player.state.volume"
                    :style="{ '--range-progress': `${player.state.volume * 100}%` }"
                    :aria-valuetext="`${Math.round(player.state.volume * 100)} percent`" :title="`Volume ${Math.round(player.state.volume * 100)}%`"
                    @input="player.setVolume(Number(($event.target as HTMLInputElement).value))" /></div>
            </div>
            <button class="library-player-download" type="button" :disabled="Boolean(player.state.downloadId)" :aria-busy="Boolean(player.state.downloadId)"
                :aria-label="`${downloadLabel} for ${title}`" @click="onDownload()"><Icon name="download" :size="17" /> {{ player.state.downloadId ? 'Preparing…' : downloadLabel }}</button>
        </div>
    </section>
</template>

<style scoped>
.library-player-bar { position: fixed; inset: auto 0 0; z-index: 60; container-type: inline-size; border-top: 1px solid var(--line); background: var(--surface); }
.library-player-inner { display: grid; grid-template-columns: minmax(170px, 250px) auto minmax(170px, 1fr) 115px auto; align-items: center; gap: 16px; width: calc(100% - 40px); min-height: 76px; margin-inline: auto; padding: 10px 0; }
.library-player-track { min-width: 0; display: flex; align-items: center; gap: 8px; }
.library-player-mark { display: flex; flex-shrink: 0; align-items: center; justify-content: center; width: 44px; height: 44px; color: var(--accent-ink); }
.library-player-copy { min-width: 0; }
.library-player-status { color: var(--accent-ink); font-size: 12px; font-weight: 500; line-height: 1.45; }
.library-player-track strong { display: block; overflow: hidden; margin-top: 3px; color: var(--ink); font-size: 14px; font-weight: 500; letter-spacing: -.15px; white-space: nowrap; text-overflow: ellipsis; }
.library-player-subtitle { overflow: hidden; margin-top: 2px; color: var(--muted); font-size: 12px; white-space: nowrap; text-overflow: ellipsis; }
.library-player-controls { display: flex; align-items: center; gap: 9px; }
.library-player-controls button { display: inline-flex; align-items: center; justify-content: center; gap: 6px; height: 44px; border-radius: 6px; font-size: 13px; font-weight: 500; }
.library-player-toggle { width: 44px; color: var(--on-accent); background: var(--accent); border-color: var(--accent) !important; border-radius: 6px !important; }
.library-player-toggle:hover { background: var(--accent-hover); }
.library-player-stop { padding-inline: 11px; color: var(--muted-strong); background: transparent; }
.library-player-stop:hover { color: var(--ink); background: var(--surface-hover); }
.library-player-position { min-width: 0; }
input[type="range"] { display: block; width: 100%; min-width: 0; height: 26px; padding: 0; appearance: none; accent-color: var(--accent); background: transparent; cursor: pointer; }
input[type="range"]::-webkit-slider-runnable-track { height: 4px; border-radius: 6px; background: linear-gradient(to right, var(--accent) var(--range-progress, 0%), var(--line) var(--range-progress, 0%)); }
input[type="range"]::-webkit-slider-thumb { width: 14px; height: 14px; margin-top: -5px; border: 2px solid var(--surface-raised); border-radius: 50%; appearance: none; background: var(--accent); box-shadow: 0 0 0 1px var(--accent); }
input[type="range"]::-moz-range-track { height: 4px; border: 0; border-radius: 6px; background: var(--line); }
input[type="range"]::-moz-range-progress { height: 4px; border-radius: 6px; background: var(--accent); }
input[type="range"]::-moz-range-thumb { width: 11px; height: 11px; border: 2px solid var(--surface-raised); border-radius: 50%; background: var(--accent); box-shadow: 0 0 0 1px var(--accent); }
input[type="range"]:disabled { opacity: .45; cursor: wait; }
.library-player-time { display: flex; align-items: center; justify-content: space-between; gap: 10px; color: var(--muted-strong); font-size: 13px; font-variant-numeric: tabular-nums; white-space: nowrap; }
.library-player-shortcut { color: var(--muted); }
kbd { padding: 1px 4px; border: 1px solid var(--line); border-radius: 6px; background: var(--surface); font-family: inherit; font-size: 12px; }
.library-player-volume { display: flex; align-items: center; gap: 9px; color: var(--muted); }
.library-player-volume input { width: 82px; }
.library-volume-toggle { display: none; }
.library-player-download { display: inline-flex; align-items: center; justify-content: center; gap: 8px; min-height: 44px; padding: 10px 15px; border-radius: 6px; color: var(--ink); background: var(--surface-raised); font-size: 13px; font-weight: 500; white-space: nowrap; }
.library-player-download:hover { border-color: var(--accent); background: var(--surface-hover); }
.library-player-download:disabled { opacity: .55; cursor: wait; }
button:focus-visible, input:focus-visible { outline: 2px solid var(--focus); outline-offset: 3px; }
.library-player-spinner { width: 16px; height: 16px; border: 2px solid currentColor; border-right-color: transparent; border-radius: 50%; animation: player-spin .8s linear infinite; }
@keyframes player-spin { to { transform: rotate(360deg); } }
@media (min-width: 1100px) { .library-player-bar { left: var(--app-sidebar-width, 232px); } }
@container (max-width: 1050px) {
    .library-player-inner { grid-template-columns: minmax(100px, 1fr) auto minmax(110px, 1fr) 44px auto; gap: 16px; padding-block: 12px; }
    .library-player-shortcut, .library-player-mark, .library-volume-icon { display: none; }
    .library-player-volume { position: relative; }
    .library-volume-toggle { display: flex; align-items: center; justify-content: center; width: 44px; height: 44px; border: 1px solid var(--line); border-radius: 6px; color: var(--muted-strong); background: var(--surface); }
    .library-volume-panel { display: none; position: absolute; bottom: 54px; right: 0; width: 152px; padding: 13px; border: 1px solid var(--line); border-radius: 6px; background: var(--surface-raised); box-shadow: 0 4px 20px color-mix(in srgb, var(--paper) 80%, transparent); }
    .library-volume-panel.is-open { display: block; }
    .library-player-volume input { width: 124px; }
}
@media (max-width: 700px) { .library-player-inner { width: calc(100% - 24px); grid-template-columns: minmax(0, 1fr) auto 44px; gap: 8px; padding-block: 11px max(11px, env(safe-area-inset-bottom)); } .library-player-controls { grid-column: 2; grid-row: 1; gap: 6px; } .library-player-stop { width: 44px; padding: 0 !important; font-size: 0 !important; gap: 0 !important; } .library-player-position { grid-column: 1; grid-row: 2; } .library-player-volume { grid-column: 3; grid-row: 1; margin-top: 0; position: relative; } .library-volume-icon { display: none; } .library-volume-toggle { display: flex; align-items: center; justify-content: center; width: 44px; height: 44px; border: 1px solid var(--line); border-radius: 6px; color: var(--muted-strong); background: var(--surface); } .library-volume-toggle:hover { color: var(--ink); background: var(--surface-hover); } .library-volume-panel { display: none; position: absolute; bottom: 54px; right: 0; width: 152px; padding: 13px; border: 1px solid var(--line); border-radius: 6px; background: var(--surface-raised); box-shadow: 0 4px 20px color-mix(in srgb, var(--paper) 80%, transparent); } .library-volume-panel.is-open { display: block; } .library-player-volume input { width: 124px; } .library-player-download { grid-column: 2 / 4; grid-row: 2; min-height: 44px; padding: 7px 10px; } .library-player-status { display: none; } .library-player-track strong { margin-top: 0; font-size: 14px; } .library-player-subtitle { margin-top: 1px; font-size: 12px; } .library-player-time { font-size: 12px; } .library-player-shortcut { display: none; } }
@media (prefers-reduced-motion: reduce) { .library-player-spinner { animation-duration: 1.5s; } }
@media (max-width: 700px) { input[type="range"] { height: 44px; } }
</style>
