<script setup lang="ts">
import { computed, ref, watch } from "vue";
const props = withDefaults(defineProps<{ id: string; label: string; value: number; min: number; max: number; step: number; scale?: number; unit?: string; bipolar?: boolean }>(), { scale: 1, unit: "", bipolar: false });
const emit = defineEmits<{ change: [value: number] }>();
const preview = ref(props.value);
watch(() => props.value, (value) => { preview.value = value; });
const display = computed(() => {
    if (props.bipolar) return preview.value === 0 ? "Center" : `${Math.round(Math.abs(preview.value) * 100)}% ${preview.value < 0 ? "L" : "R"}`;
    const number = preview.value * props.scale;
    const digits = props.step * props.scale >= 1 ? 0 : props.step * props.scale >= 0.1 ? 1 : 2;
    return `${number.toFixed(digits)}${props.unit}`;
});
const fill = computed(() => {
    const current = (preview.value - props.min) / (props.max - props.min) * 100;
    const origin = props.min < 0 && props.max > 0 ? -props.min / (props.max - props.min) * 100 : 0;
    return { '--knob-from': `${Math.min(current, origin)}%`, '--knob-to': `${Math.max(current, origin)}%` };
});
function update(event: Event) { preview.value = Number((event.target as HTMLInputElement).value); }
</script>

<template>
    <div class="studio-knob">
        <div><label :for="id">{{ label }}</label><output :for="id">{{ display }}</output></div>
        <input :id="id" type="range" :min="min" :max="max" :step="step" :value="preview" :style="fill" @input="update" @change="emit('change', preview)" />
    </div>
</template>

<style scoped>
.studio-knob > div { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 2px; }
.studio-knob label { color: var(--muted-strong); font-size: 13px; }
.studio-knob output { min-width: 48px; color: var(--muted); font-family: var(--vp-font-family-mono); font-size: 12px; font-variant-numeric: tabular-nums; text-align: right; }
.studio-knob input { display: block; width: 100%; height: 32px; margin: 0; appearance: none; background: transparent; accent-color: var(--accent); cursor: pointer; }
.studio-knob input::-webkit-slider-runnable-track { height: 5px; border-radius: 4px; background: linear-gradient(to right, var(--line) 0 var(--knob-from), var(--accent) var(--knob-from) var(--knob-to), var(--line) var(--knob-to) 100%); }
.studio-knob input::-webkit-slider-thumb { width: 13px; height: 13px; margin-top: -4px; appearance: none; border: 2px solid var(--surface-raised); border-radius: 50%; background: var(--ink); box-shadow: 0 0 0 1px var(--muted); }
.studio-knob input::-moz-range-track { height: 5px; border-radius: 4px; background: linear-gradient(to right, var(--line) 0 var(--knob-from), var(--accent) var(--knob-from) var(--knob-to), var(--line) var(--knob-to) 100%); }
.studio-knob input::-moz-range-thumb { width: 11px; height: 11px; border: 2px solid var(--surface-raised); border-radius: 50%; background: var(--ink); }
.studio-knob input:active::-webkit-slider-thumb { background: var(--focus); }
.studio-knob input:active::-moz-range-thumb { background: var(--focus); }
.studio-knob input:focus-visible { outline: 2px solid var(--focus); outline-offset: 3px; border-radius: 5px; }
@media (max-width: 1099px) { .studio-knob input { min-height: 44px; } }
</style>
