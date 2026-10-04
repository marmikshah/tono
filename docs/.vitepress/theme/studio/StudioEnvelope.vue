<script setup lang="ts">
import { computed } from "vue";
import type { Envelope } from "./model";
const props = defineProps<{ value: Envelope }>();
const points = computed(() => {
    const { a, d, s, r } = props.value;
    const hold = .5;
    const total = a + d + hold + r;
    const x = (seconds: number) => 12 + seconds / total * 376;
    const y = (amplitude: number) => 66 - amplitude * 52;
    return `12,66 ${x(a)},14 ${x(a + d)},${y(s)} ${x(a + d + hold)},${y(s)} 388,66`;
});
const description = computed(() => `Envelope: attack ${Math.round(props.value.a * 1000)} milliseconds, decay ${Math.round(props.value.d * 1000)} milliseconds, sustain ${Math.round(props.value.s * 100)} percent, release ${Math.round(props.value.r * 1000)} milliseconds.`);
</script>
<template>
    <svg class="studio-envelope-graph" viewBox="0 0 400 82" role="img" :aria-label="description">
        <path class="envelope-guide" d="M12 14H388 M12 40H388 M12 66H388" />
        <polygon class="envelope-area" :points="points" />
        <polyline class="envelope-curve" :points="points" />
        <text x="12" y="80">0</text><text x="388" y="80" text-anchor="end">ADSR</text>
    </svg>
</template>
<style scoped>
.studio-envelope-graph { display: block; width: 100%; height: 78px; overflow: visible; }
.envelope-guide { fill: none; stroke: var(--line); stroke-width: 1; vector-effect: non-scaling-stroke; }
.envelope-area { fill: var(--accent-soft); }
.envelope-curve { fill: none; stroke: var(--accent); stroke-width: 2; stroke-linejoin: round; vector-effect: non-scaling-stroke; }
.studio-envelope-graph text { fill: var(--muted); font-family: var(--vp-font-family-mono); font-size: 11px; }
</style>
