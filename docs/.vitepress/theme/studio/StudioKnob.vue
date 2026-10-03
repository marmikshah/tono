<script setup lang="ts">
import { computed, ref, watch } from "vue";
const props = withDefaults(defineProps<{ id: string; label: string; value: number; min: number; max: number; step: number; scale?: number; unit?: string }>(), { scale: 1, unit: "" });
const emit = defineEmits<{ change: [value: number] }>();
const preview = ref(props.value);
watch(() => props.value, (value) => { preview.value = value; });
const display = computed(() => {
    const number = preview.value * props.scale;
    const digits = props.step * props.scale >= 1 ? 0 : props.step * props.scale >= 0.1 ? 1 : 2;
    return `${number.toFixed(digits)}${props.unit}`;
});
function update(event: Event) { preview.value = Number((event.target as HTMLInputElement).value); }
</script>

<template>
    <div class="studio-knob">
        <div><label :for="id">{{ label }}</label><output :for="id">{{ display }}</output></div>
        <input :id="id" type="range" :min="min" :max="max" :step="step" :value="value" @input="update" @change="emit('change', preview)" />
    </div>
</template>

<style scoped>
.studio-knob > div { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 8px; }
.studio-knob label { color: #53674c; font-size: 11px; }
.studio-knob output { color: #76816c; font-family: var(--vp-font-family-mono); font-size: 9px; }
.studio-knob input { display: block; width: 100%; min-height: 21px; accent-color: #68845b; cursor: pointer; }
.studio-knob input:focus-visible { outline: 2px solid #5d8d66; outline-offset: 3px; border-radius: 4px; }
</style>
