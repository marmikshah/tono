<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(
    defineProps<{ values: number[]; progress?: number; compact?: boolean }>(),
    { progress: 0, compact: false },
);
const bars = computed(() =>
    props.compact
        ? props.values.filter((_, index) => index % 2 === 0)
        : props.values,
);
const peak = computed(() => Math.max(...bars.value, 0.001));
</script>

<template>
    <svg
        class="waveform"
        viewBox="0 0 400 120"
        preserveAspectRatio="none"
        aria-hidden="true"
    >
        <line x1="0" y1="60" x2="400" y2="60" class="wave-baseline" />
        <rect
            v-for="(value, index) in bars"
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
