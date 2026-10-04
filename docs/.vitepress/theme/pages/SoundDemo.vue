<script setup lang="ts">
import { computed, ref } from "vue";
import { withBase } from "vitepress";
import Icon from "../components/Icon.vue";
import { useAudioPlayer } from "../composables/useAudio";
import { compileProject, createStarter, type StarterId } from "../studio/model";
const presets = [
    { id: "chime", title: "Little chime", description: "Three clear bell notes with a soft, ringing tail.", detail: "Bell tones", icon: "music" },
    { id: "zap", title: "Arcade zap", description: "A quick pulse that drops from bright to low.", detail: "Arcade effect", icon: "laser" },
    { id: "beat", title: "Small beat", description: "A relaxed drum and bass loop at 110 BPM.", detail: "Drums + bass", icon: "wave" },
] as const;
const sounds = presets.map((preset) => { const project = createStarter(preset.id); return { ...preset, duration: project.duration, loop: project.loop, document: compileProject(project) }; });
const selectedId = ref<StarterId>("chime");
const selected = computed(() => sounds.find((sound) => sound.id === selectedId.value)!);
const player = useAudioPlayer();
const previewId = computed(() => `demo-${selected.value.id}`);
const playing = computed(() => player.state.activeId === previewId.value && player.state.playing);
const loading = computed(() => player.state.loadingId === previewId.value);
const progress = computed(() => player.state.activeId === previewId.value ? player.state.progress : 0);
const time = computed(() => player.state.activeId === previewId.value ? player.state.currentTime : 0);
const circumference = 2 * Math.PI * 52;
function choose(id: StarterId) { if (id === selectedId.value) return; player.stop(); selectedId.value = id; }
function play() { void player.toggleDocument(previewId.value, selected.value.document); }
</script>
<template>
    <main id="main-content" class="site-container sound-demo" tabindex="-1">
        <header class="demo-heading"><p class="demo-eyebrow">Sound demo</p><h1>A few sounds to try.</h1><p>Choose a sound. Press play.</p></header>
        <section class="demo-player" aria-label="Sound preview">
            <div class="demo-progress" aria-hidden="true"><svg viewBox="0 0 120 120"><circle class="demo-progress-track" cx="60" cy="60" r="52" /><circle class="demo-progress-fill" cx="60" cy="60" r="52" :stroke-dasharray="circumference" :stroke-dashoffset="circumference * (1 - progress)" /></svg><span><Icon :name="selected.icon" :size="34" /></span></div>
            <div class="demo-preview-copy"><span class="demo-sound-kind">{{ selected.detail }}</span><h2>{{ selected.title }}</h2><p>{{ selected.description }}</p></div>
            <div class="demo-preview-actions"><button class="demo-play" type="button" :aria-busy="loading" :aria-label="loading ? `Cancel ${selected.title} preview` : playing ? `Pause ${selected.title}` : `Play ${selected.title}`" @click="play"><span v-if="loading" class="demo-spinner" aria-hidden="true" /><Icon v-else :name="playing ? 'pause' : 'play'" :size="18" />{{ loading ? 'Cancel' : playing ? 'Pause' : 'Play sound' }}</button><span class="demo-time">{{ time.toFixed(2) }} <span>/ {{ selected.duration.toFixed(2) }}s</span><span v-if="selected.loop" class="demo-loop">Loop</span></span></div>
        </section>
        <p v-if="player.state.error" class="demo-error" role="alert">{{ player.state.error }}</p>
        <div class="demo-presets" role="group" aria-label="Sound demos"><button v-for="sound in sounds" :key="sound.id" type="button" :aria-label="`Choose ${sound.title}`" :aria-pressed="selectedId === sound.id" @click="choose(sound.id)"><span class="demo-preset-icon"><Icon :name="sound.icon" :size="20" /></span><span><strong>{{ sound.title }}</strong><small>{{ sound.detail }}</small></span><span class="demo-preset-duration">{{ sound.duration.toFixed(sound.duration < 1 ? 1 : 0) }}s<Icon v-if="selectedId === sound.id" name="check" :size="16" /></span></button></div>
        <nav class="demo-links" aria-label="Explore tono"><a :href="withBase('/sounds')">Sound effects<Icon name="arrow" :size="16" /></a><a :href="withBase('/bgm')">Background music<Icon name="arrow" :size="16" /></a><a :href="withBase('/showcase')">Examples<Icon name="arrow" :size="16" /></a></nav>
        <p class="demo-desktop-note">The full Sound Studio is available on desktop.</p>
    </main>
</template>
<style scoped>
.sound-demo { width: calc(100% - 32px); max-width: 740px; margin-inline: auto; padding-block: 28px 32px; color: var(--ink); }
.demo-eyebrow { margin-bottom: 8px !important; color: var(--accent-ink); font-size: 12px; font-weight: 500; }
.demo-heading h1 { font-size: 28px; font-weight: 600; letter-spacing: -.7px; line-height: 1.2; }
.demo-heading > p:last-child { margin-top: 10px; color: var(--muted); font-size: 14px; }
.demo-player { display: grid; grid-template-columns: 120px minmax(0, 1fr); align-items: center; gap: 18px 24px; min-height: 190px; margin-top: 24px; padding: 22px; border: 1px solid var(--line); border-radius: 14px; background: var(--surface); }
.demo-progress { position: relative; grid-row: span 2; width: 120px; height: 120px; }
.demo-progress svg { width: 100%; height: 100%; overflow: visible; transform: rotate(-90deg); }
.demo-progress circle { fill: none; stroke-width: 3px; }
.demo-progress-track { stroke: var(--line); }
.demo-progress-fill { stroke: var(--accent); stroke-linecap: round; }
.demo-progress > span { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; color: var(--accent-ink); }
.demo-progress > span > svg { width: 34px; height: 34px; transform: none; }
.demo-preview-copy { min-width: 0; }
.demo-sound-kind { color: var(--muted); font-size: 12px; }
.demo-preview-copy h2 { margin-top: 5px; font-size: 22px; line-height: 1.25; font-weight: 550; letter-spacing: -.45px; }
.demo-preview-copy > p { max-width: 350px; margin-top: 8px; color: var(--muted-strong); font-size: 14px; line-height: 1.5; }
.demo-preview-actions { display: flex; align-items: center; flex-wrap: wrap; gap: 16px; }
.demo-play { display: inline-flex; align-items: center; justify-content: center; gap: 8px; min-width: 132px; min-height: 48px; padding: 10px 15px; border-radius: 6px; background: var(--accent); color: var(--on-accent); font-size: 14px; font-weight: 600; cursor: pointer; }
.demo-play:hover { background: var(--accent-hover); }
.demo-time { display: inline-flex; align-items: center; gap: 5px; color: var(--ink); font-family: var(--vp-font-family-mono); font-size: 12px; font-variant-numeric: tabular-nums; white-space: nowrap; }
.demo-time > span { color: var(--muted); }
.demo-time .demo-loop { margin-left: 5px; font-family: var(--vp-font-family-base); }
.demo-presets { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 8px; margin-top: 18px; }
.demo-presets > button { display: flex; align-items: center; gap: 10px; min-width: 0; min-height: 76px; padding: 12px; border: 1px solid var(--line); border-radius: 8px; background: transparent; color: var(--ink); text-align: left; cursor: pointer; }
.demo-presets > button:hover { background: var(--surface-hover); }
.demo-presets > button[aria-pressed="true"] { border-color: var(--accent); background: var(--accent-soft); }
.demo-preset-icon { display: flex; flex-shrink: 0; color: var(--muted); }
.demo-presets > button[aria-pressed="true"] .demo-preset-icon { color: var(--accent-ink); }
.demo-presets strong { display: block; font-size: 13px; font-weight: 550; line-height: 1.4; }
.demo-presets small { display: block; margin-top: 3px; color: var(--muted); font-size: 12px; white-space: nowrap; }
.demo-preset-duration { display: flex; align-items: center; gap: 8px; margin-left: auto; color: var(--muted); font-family: var(--vp-font-family-mono); font-size: 12px; }
.demo-preset-duration > svg { color: var(--accent-ink); }
.demo-links { display: flex; flex-wrap: wrap; gap: 8px 24px; margin-top: 22px; padding-top: 12px; border-top: 1px solid var(--line); }
.demo-links a { display: inline-flex; align-items: center; justify-content: space-between; gap: 10px; min-height: 44px; color: var(--muted-strong); font-size: 13px; }
.demo-links a:hover { color: var(--accent-ink); }
.demo-desktop-note { margin-top: 12px; color: var(--muted); font-size: 12px; }
.demo-error { margin-top: 12px; padding: 10px 12px; border-radius: 6px; background: var(--danger-soft); color: var(--danger); font-size: 13px; }
.sound-demo :is(button, a):focus-visible { outline: 2px solid var(--focus); outline-offset: 3px; }
.demo-spinner { width: 16px; height: 16px; border: 2px solid color-mix(in srgb, var(--on-accent) 40%, transparent); border-top-color: var(--on-accent); border-radius: 50%; animation: demo-spin .7s linear infinite; }
@keyframes demo-spin { to { transform: rotate(360deg); } }
@media (max-width: 700px) {
    .sound-demo { padding-top: 24px; }
    .demo-player { grid-template-columns: 82px minmax(0, 1fr); gap: 20px 18px; padding: 20px 16px; }
    .demo-progress { grid-row: auto; width: 82px; height: 82px; }
    .demo-preview-copy h2 { font-size: 20px; }
    .demo-preview-copy > p { font-size: 13px; }
    .demo-preview-actions { grid-column: 1 / -1; justify-content: space-between; gap: 12px; }
    .demo-time { display: block; text-align: right; }
    .demo-time .demo-loop { display: block; margin-top: 4px; margin-left: 0; }
    .demo-presets { grid-template-columns: minmax(0, 1fr); }
    .demo-presets > button { min-height: 64px; padding: 10px 14px; }
    .demo-presets strong { font-size: 14px; }
    .demo-links { gap: 0; }
    .demo-links a { width: 100%; }
}
@media (max-width: 370px) { .demo-player { grid-template-columns: 66px minmax(0, 1fr); column-gap: 12px; padding-inline: 12px; } .demo-progress { width: 66px; height: 66px; } .demo-preview-copy h2 { font-size: 19px; } }
@media (prefers-reduced-motion: reduce) { .demo-spinner { animation-duration: 1.5s; } }
</style>
