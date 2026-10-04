<script setup lang="ts">
import { computed, useId } from "vue";

const props = withDefaults(
    defineProps<{ values: number[]; progress?: number; compact?: boolean; continuous?: boolean }>(),
    { progress: 0, compact: false, continuous: false },
);
const bars = computed(() =>
    props.compact
        ? props.values.filter((_, index) => index % 2 === 0)
        : props.values,
);
const peak = computed(() => Math.max(...bars.value, 0.001));
const clipId = `wave-progress-${useId().replace(/:/g, "-")}`;
const progressWidth = computed(() => Math.max(0, Math.min(1, props.progress)) * 400);
const shape = computed(() => {
    if (!bars.value.length) return "M0 59 L400 59 L400 61 L0 61 Z";
    const upper = bars.value.map((value, index) => {
        const x = bars.value.length === 1 ? 200 : index * 400 / (bars.value.length - 1);
        const height = Math.max(1, value / peak.value * 46);
        return [x, 60 - height];
    });
    const lower = [...upper].reverse().map(([x, y]) => [x, 120 - y]);
    return `M0 60 ${upper.map(([x, y]) => `L${x.toFixed(2)} ${y.toFixed(2)}`).join(" ")} L400 60 ${lower.map(([x, y]) => `L${x.toFixed(2)} ${y.toFixed(2)}`).join(" ")} Z`;
});
</script>

<template>
    <svg
        class="waveform"
        viewBox="0 0 400 120"
        preserveAspectRatio="none"
        aria-hidden="true"
    >
        <line x1="0" y1="60" x2="400" y2="60" class="wave-baseline" />
        <template v-if="continuous">
            <defs><clipPath :id="clipId"><rect x="0" y="0" :width="progressWidth" height="120" /></clipPath></defs>
            <path :d="shape" class="wave-shape" />
            <path v-if="progress > 0" :d="shape" class="wave-shape wave-played" :clip-path="`url(#${clipId})`" />
        </template>
        <rect
            v-for="(value, index) in continuous ? [] : bars"
            :key="index"
            :x="(index * 400) / bars.length + 1"
            :y="60 - Math.max(2, (value / peak) * 94) / 2"
            :width="Math.max(2, 400 / bars.length - 2)"
            :height="Math.max(2, (value / peak) * 94)"
            rx="1.5"
            :class="{
                'wave-played': index / bars.length <= progress && progress > 0,
            }"
        />
    </svg>
</template>

<style scoped>
.wave-shape { fill: currentColor; opacity: .46; }
.wave-shape.wave-played { opacity: 1; }
</style>
