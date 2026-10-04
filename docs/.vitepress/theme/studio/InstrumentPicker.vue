<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import Icon from "../components/Icon.vue";
import { instruments, type InstrumentId } from "./model";

defineProps<{ full: boolean }>();
const emit = defineEmits<{ add: [instrument: InstrumentId]; close: []; drag: [event: DragEvent, instrument: InstrumentId] }>();
const panel = ref<HTMLElement>();
const searchField = ref<HTMLInputElement>();
const search = ref("");
const category = ref("All");
const above = ref(false);
const placement = ref<Record<string, string>>({});
const categories = ["All", "Tones", "Drums"];
const filtered = computed(() => instruments.filter((instrument) => {
    if (category.value === "Drums" && !instrument.drum) return false;
    if (category.value === "Tones" && instrument.drum) return false;
    return `${instrument.name} ${instrument.detail}`.toLowerCase().includes(search.value.trim().toLowerCase());
}));
function outside(event: PointerEvent) {
    const target = event.target as HTMLElement;
    if (!panel.value?.contains(target) && !target.closest('[aria-controls="studio-instrument-picker"]')) emit("close");
}
function fitViewport() {
    const element = panel.value;
    if (!element || window.innerWidth <= 700) { above.value = false; placement.value = {}; return; }
    const anchor = element.parentElement!.getBoundingClientRect();
    const header = parseFloat(getComputedStyle(element).getPropertyValue("--site-header-height")) || 76;
    const topEdge = header + 12;
    const belowSpace = window.innerHeight - anchor.bottom - 10 - 16;
    const aboveSpace = anchor.top - 10 - topEdge;
    // Short windows need the full viewport height rather than a tiny anchored menu.
    if (Math.max(belowSpace, aboveSpace) < 250) {
        const width = element.getBoundingClientRect().width;
        above.value = false;
        placement.value = { position: "fixed", top: `${topEdge}px`, bottom: "auto", left: `${Math.max(16, Math.min(anchor.right - width, window.innerWidth - width - 16))}px`, right: "auto", maxHeight: `${Math.max(0, window.innerHeight - topEdge - 16)}px` };
        return;
    }
    above.value = belowSpace < 300 && aboveSpace > belowSpace;
    placement.value = { maxHeight: `${Math.max(0, Math.min(650, window.innerHeight * .75, above.value ? aboveSpace : belowSpace))}px` };
}
onMounted(async () => {
    document.addEventListener("pointerdown", outside);
    window.addEventListener("resize", fitViewport);
    window.addEventListener("scroll", fitViewport, { passive: true });
    await nextTick();
    fitViewport();
    await nextTick();
    searchField.value?.focus({ preventScroll: true });
});
onBeforeUnmount(() => { document.removeEventListener("pointerdown", outside); window.removeEventListener("resize", fitViewport); window.removeEventListener("scroll", fitViewport); });
</script>

<template>
    <section id="studio-instrument-picker" ref="panel" class="instrument-picker" :class="{ 'is-above': above }" :style="placement" role="dialog" aria-label="Add an instrument" @keydown.esc.stop.prevent="emit('close')">
        <div class="picker-heading"><h3>Add an instrument</h3><button type="button" aria-label="Close instrument picker" @click="emit('close')"><Icon name="close" :size="20" /></button></div>
        <label class="picker-search"><span class="sr-only">Find an instrument</span><input ref="searchField" v-model="search" type="search" placeholder="Search instruments…" autocomplete="off" /></label>
        <div class="picker-tabs" role="group" aria-label="Instrument types"><button v-for="option in categories" :key="option" type="button" :aria-pressed="category === option" @click="category = option">{{ option }}</button></div>
        <p v-if="full" class="picker-full">Your mix has eight layers. Remove one to add another.</p>
        <div class="picker-instruments">
            <button v-for="instrument in filtered" :key="instrument.id" type="button" draggable="true" :disabled="full" :aria-label="`Add ${instrument.name} instrument`" @click="emit('add', instrument.id)" @dragstart="emit('drag', $event, instrument.id)">
                <span class="picker-symbol"><Icon :name="instrument.drum ? 'impact' : instrument.id === 'noise' ? 'layers' : 'wave'" :size="21" /></span>
                <span><strong>{{ instrument.name }}</strong><small>{{ instrument.detail }}</small></span><span class="picker-plus" aria-hidden="true">+</span>
            </button>
        </div>
        <p v-if="!filtered.length" class="picker-empty">No instruments match that search.</p>
    </section>
</template>

<style scoped>
.instrument-picker { position: absolute; top: calc(100% + 10px); right: 0; z-index: 40; width: 430px; max-height: min(650px, 75vh); padding: 18px; overflow-y: auto; overscroll-behavior: contain; border: 1px solid var(--line); border-radius: 6px; background: var(--surface); box-shadow: 0 16px 45px color-mix(in srgb, var(--paper) 90%, transparent); }
.picker-heading { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
.picker-heading h3 { color: var(--ink); font-family: var(--vp-font-family-base); font-size: 18px; font-weight: 650; }
.picker-heading button { display: flex; align-items: center; justify-content: center; width: 36px; height: 36px; border-radius: 7px; color: var(--muted-strong); }
.picker-heading button:hover { background: var(--surface-hover); color: var(--ink); }
.picker-search { display: block; margin-top: 14px; }
.picker-search input { width: 100%; min-height: 44px; padding: 10px 12px; border: 1px solid var(--line); border-radius: 3px; background: var(--paper); color: var(--ink); font: inherit; font-size: 14px; }
.picker-search input::placeholder { color: var(--muted); }
.instrument-picker :is(input, button):focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
.picker-tabs { display: flex; gap: 14px; margin-block: 10px 6px; border-bottom: 1px solid var(--line); }
.picker-tabs button { min-height: 38px; padding: 5px 2px; border-bottom: 2px solid transparent; color: var(--muted); font-size: 14px; }
.picker-tabs button:hover { background: var(--surface-hover); }
.picker-tabs button[aria-pressed="true"] { border-bottom-color: var(--accent); color: var(--ink); }
.picker-instruments { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); column-gap: 14px; }
.picker-instruments button { display: flex; align-items: center; gap: 10px; min-width: 0; min-height: 70px; padding: 10px 3px; border-bottom: 1px solid var(--line); background: transparent; color: var(--ink); text-align: left; cursor: grab; }
.picker-instruments button:hover { background: var(--surface-hover); }
.picker-symbol { display: flex; align-items: center; flex-shrink: 0; color: var(--accent-ink); }
.picker-instruments strong { display: block; font-size: 14px; font-weight: 600; line-height: 1.4; }
.picker-instruments small { display: block; margin-top: 4px; color: var(--muted); font-size: 12px; line-height: 1.5; }
.picker-plus { margin-left: auto; color: var(--muted); font-size: 21px; }
.picker-instruments button:hover .picker-plus { color: var(--accent-ink); }
.picker-instruments button:disabled { opacity: .45; cursor: default; }
.picker-empty, .picker-full { margin-top: 14px; color: var(--muted); font-size: 12px; line-height: 1.6; }
.picker-full { margin-bottom: 13px; color: var(--danger); }
@media (min-width: 701px) { .instrument-picker.is-above { top: auto; bottom: calc(100% + 10px); } }
@media (max-width: 1099px) { .picker-heading button { width: 44px; height: 44px; } .picker-tabs button { min-width: 44px; min-height: 44px; } }
@media (max-width: 700px) {
    .instrument-picker { position: fixed; top: calc(var(--site-header-height, 64px) + 105px); left: 16px; right: 16px; width: auto; max-height: calc(100dvh - var(--site-header-height, 64px) - 125px); padding: 18px; }
    .picker-instruments { grid-template-columns: 1fr; }
    .picker-instruments button { min-height: 68px; padding: 11px 13px; }
    .picker-instruments small { font-size: 13px; }
}
</style>
