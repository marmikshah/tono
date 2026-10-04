<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useAudioPlayer } from "../composables/useAudio";
import Icon from "../components/Icon.vue";
import StudioKnob from "../studio/StudioKnob.vue";
import StudioEnvelope from "../studio/StudioEnvelope.vue";
import InstrumentPicker from "../studio/InstrumentPicker.vue";
import { cloneProject, compileProject, createLayer, createStarter, DRAFT_KEY, exportProject, importProject, instrumentFor, instruments, MAX_LAYERS, parseNotes, projectFilename, validateProject, type Envelope, type InstrumentId, type StarterId, type StudioLayer, type StudioProject } from "../studio/model";

const player = useAudioPlayer();
const previewId = "studio-preview";
const project = ref<StudioProject>(createStarter());
const selectedId = ref(project.value.layers[0].id);
const selectedStep = ref(0);
const brush = ref("C5");
const undoStack = ref<StudioProject[]>([]);
const redoStack = ref<StudioProject[]>([]);
const fileInput = ref<HTMLInputElement>();
const mixPanel = ref<HTMLElement>();
const addButton = ref<HTMLButtonElement>();
const pickerOpen = ref(false);
const browserSearch = ref("");
const utilities = ref<HTMLDialogElement>();
const utilitiesButton = ref<HTMLButtonElement>();
const deviceTabs = [{ id: "tone", label: "Tone" }, { id: "envelope", label: "Envelope" }, { id: "effects", label: "Effects" }, { id: "pattern", label: "Pattern" }] as const;
const activeTab = ref<typeof deviceTabs[number]["id"]>("tone");
const dockCollapsed = ref(false);
const shortWindow = ref(false);
const compactEditor = ref(false);
const dockReady = ref(false);
const activeStarter = ref<StarterId | "">("chime");
const exportedState = ref("");
const draftState = ref("Local draft");
const renderedSource = ref("");
let restoring = false;
let compactQuery: MediaQueryList | undefined;
let shortQuery: MediaQueryList | undefined;
const editorError = ref("");
const status = ref("Your draft saves in this browser.");
const dropActive = ref(false);
const selected = computed(() => project.value.layers.find((layer) => layer.id === selectedId.value));
const browserInstruments = computed(() => instruments.filter((instrument) => `${instrument.name} ${instrument.detail}`.toLowerCase().includes(browserSearch.value.trim().toLowerCase())));
const browserGroups = computed(() => [{ name: "Synths", ids: ["sine", "pulse", "saw", "fm", "bass", "noise"] }, { name: "Tonal", ids: ["bell", "pluck", "piano", "flute", "strings"] }, { name: "Drums", ids: ["kick", "snare", "hat"] }].map((group) => ({ name: group.name, instruments: browserInstruments.value.filter((instrument) => group.ids.includes(instrument.id)) })).filter((group) => group.instruments.length));
const compiled = computed(() => {
    try { return { document: compileProject(project.value), error: "" }; }
    catch (error) { return { document: undefined, error: error instanceof Error ? error.message : "Check your sound settings." }; }
});
const errorText = computed(() => editorError.value || compiled.value.error || player.state.error);
const playing = computed(() => player.state.activeId === previewId && player.state.playing);
const loading = computed(() => player.state.loadingId === previewId);
const audible = computed(() => project.value.layers.some((layer) => !layer.mute && layer.gain > 0 && layer.steps.some((step) => step.trim())));
const activeNotes = computed(() => project.value.layers.reduce((sum, layer) => sum + layer.steps.filter(Boolean).length, 0));
const progress = computed(() => player.state.activeId === previewId ? player.state.progress : 0);
const barSeconds = computed(() => 240 / project.value.tempo);
const stepSeconds = computed(() => 60 / project.value.tempo / 4);
const playTime = computed(() => player.state.activeId === previewId ? Math.min(player.state.currentTime, project.value.duration) : 0);
const playheadStep = computed(() => player.state.activeId === previewId && (playing.value || progress.value > 0) && progress.value < 1 ? Math.floor(playTime.value / stepSeconds.value) % 16 : -1);
const exportState = computed(() => !exportedState.value ? "Not exported" : exportedState.value === JSON.stringify(project.value) ? "Project exported" : "Changed since export");
const canSeek = computed(() => Boolean(compiled.value.document && renderedSource.value === JSON.stringify(compiled.value.document) && player.state.duration > 0 && !loading.value));
const starters: { id: StarterId; label: string }[] = [{ id: "chime", label: "Little chime" }, { id: "zap", label: "Arcade zap" }, { id: "beat", label: "Small beat" }, { id: "blank", label: "Blank canvas" }];
const noteValue = (event: Event) => (event.target as HTMLInputElement).value;
const numberValue = (event: Event) => Number(noteValue(event));
// Lazy inputs keep a focused draft through playback updates, then commit one edit.
const projectNameField = computed({ get: () => project.value.name, set: (value: string) => setGlobal("name", value) });
const tempoField = computed({ get: () => project.value.tempo, set: (value: number) => setGlobal("tempo", Number(value)) });
const durationField = computed({ get: () => Number(project.value.duration.toFixed(3)), set: (value: number) => setGlobal("duration", Number(value)) });
const seedField = computed({ get: () => project.value.seed, set: (value: number) => setGlobal("seed", Number(value)) });
const layerNameField = computed({ get: () => selected.value?.name || "", set: (value: string) => setLayer("name", value) });
const stepNoteField = computed({ get: () => selected.value?.steps[selectedStep.value] || "", set: (value: string) => editNotes(value) });

function selectLayer(id: string, resetBrush = false) {
    const changed = id !== selectedId.value;
    selectedId.value = id;
    const layer = project.value.layers.find((layer) => layer.id === id);
    if (changed || resetBrush) brush.value = layer?.steps.find(Boolean) || (layer && instrumentFor(layer.instrument).drum ? instrumentFor(layer.instrument).name.replace("Hi-hat", "Hat") : "C4");
}
async function editLayer(id: string) {
    selectLayer(id);
}
function closePicker(restoreFocus = true) { pickerOpen.value = false; if (restoreFocus) addButton.value?.focus(); }
function reconcileSelection(resetBrush = false) {
    const id = project.value.layers.some((layer) => layer.id === selectedId.value) ? selectedId.value : project.value.layers[0]?.id || "";
    selectLayer(id, resetBrush);
}
function commit(change: (project: StudioProject) => void) {
    try {
        const next = cloneProject(project.value);
        change(next);
        const validated = validateProject(next);
        if (JSON.stringify(validated) === JSON.stringify(project.value)) { editorError.value = ""; return; }
        undoStack.value.push(cloneProject(project.value));
        if (undoStack.value.length > 60) undoStack.value.shift();
        redoStack.value = [];
        project.value = validated;
        activeStarter.value = "";
        editorError.value = "";
        reconcileSelection();
    } catch (error) { editorError.value = error instanceof Error ? error.message : "Check this setting."; }
}
function setGlobal<K extends keyof StudioProject>(key: K, value: StudioProject[K]) { commit((project) => { project[key] = value; }); }
function setLayer<K extends keyof StudioLayer>(key: K, value: StudioLayer[K]) {
    const id = selectedId.value;
    commit((project) => { const layer = project.layers.find((layer) => layer.id === id); if (layer) layer[key] = value; });
}
function setEnvelope(key: keyof Envelope, value: number) {
    const id = selectedId.value;
    commit((project) => { const layer = project.layers.find((layer) => layer.id === id); if (layer) layer.env[key] = value; });
}
async function addLayer(instrument: InstrumentId, before?: string) {
    if (project.value.layers.length >= MAX_LAYERS) { editorError.value = `Your mix has room for ${MAX_LAYERS} instruments. Remove a layer to add another.`; return; }
    let number = 1;
    while (project.value.layers.some((layer) => layer.id === `layer_${number}`)) number++;
    const layer = createLayer(instrument, `layer_${number}`);
    commit((project) => { const at = before ? project.layers.findIndex((layer) => layer.id === before) : -1; project.layers.splice(at < 0 ? project.layers.length : at, 0, layer); });
    selectLayer(layer.id);
    selectedStep.value = 0;
    closePicker(false);
    await nextTick();
    const card = mixPanel.value?.querySelector<HTMLElement>(`[data-layer="${layer.id}"]`);
    card?.querySelector<HTMLElement>(".studio-layer-select")?.focus({ preventScroll: true });
    card?.scrollIntoView({ block: "nearest" });
}
function removeLayer(id: string) { commit((project) => { project.layers = project.layers.filter((layer) => layer.id !== id); }); }
function muteLayer(id: string) { commit((project) => { const layer = project.layers.find((layer) => layer.id === id); if (layer) layer.mute = !layer.mute; }); }
function moveLayer(id: string, target: number) {
    commit((project) => { const from = project.layers.findIndex((layer) => layer.id === id); if (from < 0 || target < 0 || target >= project.layers.length) return; const [layer] = project.layers.splice(from, 1); project.layers.splice(target, 0, layer); });
}
function selectStep(layer: StudioLayer, step: number) {
    selectLayer(layer.id);
    selectedStep.value = step;
    const note = brush.value;
    if (!layer.steps[step]) commit((project) => { project.layers.find((entry) => entry.id === layer.id)!.steps[step] = note; });
}
function clearStep(layer: StudioLayer, step: number) {
    selectLayer(layer.id); selectedStep.value = step;
    commit((project) => { project.layers.find((entry) => entry.id === layer.id)!.steps[step] = ""; });
}
async function stepKeyboard(event: KeyboardEvent, layer: StudioLayer, step: number) {
    if (event.key === "Delete" || event.key === "Backspace") { event.preventDefault(); event.stopPropagation(); clearStep(layer, step); return; }
    const direction = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : event.key === "ArrowDown" ? 4 : event.key === "ArrowUp" ? -4 : 0;
    if (!direction && !["Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const next = event.key === "Home" ? 0 : event.key === "End" ? 15 : (step + direction + 16) % 16;
    selectLayer(layer.id); selectedStep.value = next;
    await nextTick();
    mixPanel.value?.querySelector<HTMLElement>(`[data-layer="${layer.id}"] [data-step="${next}"]`)?.focus();
}
function isPlayhead(layer: StudioLayer, step: number) { return !layer.mute && (layer.repeat || playTime.value < barSeconds.value) && playheadStep.value === step; }
function noteLabel(note: string) { return note.trim().split(/[\s+,]+/).filter(Boolean)[0] || ""; }
function chordCount(note: string) { return Math.max(0, note.trim().split(/[\s+,]+/).filter(Boolean).length - 1); }
function editNotes(text: string) {
    try { parseNotes(text); setLayer("steps", selected.value!.steps.map((note, index) => index === selectedStep.value ? text.trim() : note)); if (text.trim()) brush.value = text.trim(); }
    catch (error) { editorError.value = error instanceof Error ? error.message : "Check this note."; }
}
function clearPattern() { setLayer("steps", Array<string>(16).fill("")); }
function undo() { const previous = undoStack.value.pop(); if (!previous) return; redoStack.value.push(cloneProject(project.value)); project.value = previous; activeStarter.value = ""; editorError.value = ""; reconcileSelection(true); }
function redo() { const next = redoStack.value.pop(); if (!next) return; undoStack.value.push(cloneProject(project.value)); project.value = next; activeStarter.value = ""; editorError.value = ""; reconcileSelection(true); }
function starter(id: StarterId) { const next = createStarter(id); commit((project) => { Object.assign(project, next); }); activeStarter.value = id; selectedStep.value = 0; reconcileSelection(true); }
function dragInstrument(event: DragEvent, id: InstrumentId) { event.dataTransfer?.setData("application/x-tono-instrument", id); if (event.dataTransfer) event.dataTransfer.effectAllowed = "copy"; }
function dragLayer(event: DragEvent, id: string) { event.dataTransfer?.setData("application/x-tono-layer", id); if (event.dataTransfer) event.dataTransfer.effectAllowed = "move"; }
function drop(event: DragEvent, before?: string) {
    dropActive.value = false;
    const instrument = event.dataTransfer?.getData("application/x-tono-instrument");
    if (instruments.some((entry) => entry.id === instrument)) { addLayer(instrument as InstrumentId, before); return; }
    const id = event.dataTransfer?.getData("application/x-tono-layer");
    if (id) moveLayer(id, before ? project.value.layers.findIndex((layer) => layer.id === before) : project.value.layers.length - 1);
}
function saveProject() {
    try {
        const url = URL.createObjectURL(new Blob([exportProject(project.value)], { type: "application/json" }));
        const link = document.createElement("a"); link.href = url; link.download = `${projectFilename(project.value)}.tono.json`; link.click();
        setTimeout(() => URL.revokeObjectURL(url), 1000);
        exportedState.value = JSON.stringify(project.value);
        status.value = "Project saved. Open it here whenever you want to keep creating.";
    } catch (error) { editorError.value = error instanceof Error ? error.message : "This project could not save."; }
}
async function openProject(event: Event) {
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    try {
        if (file.size > 1_000_000) throw new Error("Choose a project file smaller than 1 MB.");
        const next = importProject(await file.text());
        commit((project) => { Object.assign(project, next); });
        exportedState.value = JSON.stringify(next);
        selectedStep.value = 0; reconcileSelection(true); status.value = "Project opened. Pick up where you left off.";
    } catch (error) { editorError.value = error instanceof Error ? error.message : "This project could not open."; }
    finally { input.value = ""; }
}
function keyboard(event: KeyboardEvent) {
    if (document.querySelector('[aria-modal="true"]:not(dialog), dialog[open][aria-modal="true"]')) return;
    const target = event.target as HTMLElement;
    if (["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName) || target.isContentEditable) return;
    if (event.code === "Space" && !event.ctrlKey && !event.metaKey && !event.altKey) {
        if (["BUTTON", "A", "SUMMARY"].includes(target.tagName) && !target.classList.contains("studio-step")) return;
        event.preventDefault(); if (!event.repeat) void play(); return;
    }
    if (!(event.ctrlKey || event.metaKey)) return;
    if (event.key.toLowerCase() === "z") { event.preventDefault(); if (event.shiftKey) redo(); else undo(); }
    if (event.key.toLowerCase() === "y") { event.preventDefault(); redo(); }
}
async function play() {
    const document = compiled.value.document;
    if (!document || !audible.value) return;
    const source = JSON.stringify(document);
    await player.toggleDocument(previewId, document);
    if (playing.value) renderedSource.value = source;
}
function seek(event: Event) { if (canSeek.value) player.seek(numberValue(event) / project.value.duration); }
function download() { if (compiled.value.document) void player.downloadDocument(previewId, compiled.value.document, `${projectFilename(project.value)}.wav`); }
watch(project, () => {
    player.stop();
    renderedSource.value = "";
    if (restoring) return;
    try { localStorage.setItem(DRAFT_KEY, JSON.stringify(project.value)); draftState.value = "Saved locally"; status.value = "Changes saved on this device."; }
    catch { draftState.value = "Draft not saved"; status.value = "Save a project copy to keep these changes."; }
});
function responsiveInspector(event: MediaQueryListEvent | MediaQueryList) { compactEditor.value = event.matches; }
function responsiveHeight(event: MediaQueryListEvent | MediaQueryList) { shortWindow.value = event.matches; dockCollapsed.value = event.matches; }
function openUtilities() { utilities.value?.showModal(); }
function closeUtilities() { utilities.value?.close(); }
function utilitiesBackdrop(event: MouseEvent) {
    if (event.target !== utilities.value || !utilities.value) return;
    const bounds = utilities.value.getBoundingClientRect();
    if (event.clientX < bounds.left || event.clientX > bounds.right || event.clientY < bounds.top || event.clientY > bounds.bottom) closeUtilities();
}
async function tabKeyboard(event: KeyboardEvent, index: number) {
    const offset = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    if (!offset && !["Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const next = event.key === "Home" ? 0 : event.key === "End" ? deviceTabs.length - 1 : (index + offset + deviceTabs.length) % deviceTabs.length;
    activeTab.value = deviceTabs[next].id;
    await nextTick();
    document.getElementById(`studio-tab-${activeTab.value}`)?.focus();
}
onMounted(async () => {
    window.addEventListener("keydown", keyboard);
    compactQuery = window.matchMedia("(max-width: 1099px)");
    responsiveInspector(compactQuery);
    compactQuery.addEventListener("change", responsiveInspector);
    shortQuery = window.matchMedia("(max-height: 639px)");
    responsiveHeight(shortQuery);
    shortQuery.addEventListener("change", responsiveHeight);
    try {
        const draft = localStorage.getItem(DRAFT_KEY);
        if (draft) { const restored = validateProject(JSON.parse(draft)); restoring = true; project.value = restored; activeStarter.value = ""; reconcileSelection(true); draftState.value = "Draft restored"; status.value = "Your last local draft is ready."; await nextTick(); restoring = false; }
    } catch { /* Invalid or old drafts never replace the working starter. */ }
    dockReady.value = true;
});
onBeforeUnmount(() => { compactQuery?.removeEventListener("change", responsiveInspector); shortQuery?.removeEventListener("change", responsiveHeight); utilities.value?.close(); window.removeEventListener("keydown", keyboard); });
</script>

<template>
    <main id="main-content" class="site-container studio-page" tabindex="-1">
        <header class="studio-project-top">
            <h1 :aria-label="project.name"><label class="studio-name-field"><span class="sr-only">Sound name</span><input v-model.lazy="projectNameField" maxlength="80" /></label></h1>
            <select class="studio-preset-select" aria-label="Starting sound" :value="activeStarter" @change="starter(noteValue($event) as StarterId)"><option value="" disabled>Starters</option><option v-for="preset in starters" :key="preset.id" :value="preset.id">{{ preset.label }}</option></select>
            <span class="studio-draft-state"><i aria-hidden="true" />{{ draftState }}</span>
            <div class="studio-project-actions">
                <button type="button" aria-label="Undo last edit" title="Undo (Ctrl / ⌘ Z)" :disabled="!undoStack.length" @click="undo"><Icon name="undo" :size="16" /></button>
                <button type="button" aria-label="Redo last edit" title="Redo (Ctrl / ⌘ Shift Z)" :disabled="!redoStack.length" @click="redo"><Icon name="redo" :size="16" /></button>
                <span class="studio-action-divider" aria-hidden="true" />
                <button type="button" aria-label="Open project" title="Open project" @click="fileInput?.click()"><Icon name="layers" :size="16" /><span>Open <span class="studio-file-label-detail">project</span></span></button>
                <button type="button" aria-label="Save project" title="Save project" @click="saveProject"><Icon name="download" :size="16" /><span>Save <span class="studio-file-label-detail">project</span></span></button>
                <button ref="utilitiesButton" type="button" aria-label="Project utilities" aria-haspopup="dialog" title="Seed and sound recipe" @click="openUtilities"><Icon name="settings" :size="16" /></button>
            </div>
            <input ref="fileInput" class="sr-only" type="file" accept="application/json,.json" aria-label="Open studio project JSON" @change="openProject" />
        </header>
        <section class="studio-transport" aria-label="Sound playback and export">
            <div class="studio-transport-main">
                <button class="studio-play" type="button" :disabled="!audible || !compiled.document" :aria-busy="loading" :aria-label="loading ? 'Cancel rendering your sound' : playing ? 'Pause your sound' : 'Play your sound'" @click="play"><span v-if="loading" class="studio-spinner" aria-hidden="true" /><Icon v-else :name="playing ? 'pause' : 'play'" :size="16" />{{ loading ? 'Cancel' : playing ? 'Pause' : 'Play' }}</button>
                <button class="studio-stop" type="button" aria-label="Stop your sound" title="Stop and return to the beginning" :disabled="player.state.activeId !== previewId && !loading" @click="player.stop()"><Icon name="stop" :size="15" /></button>
                <span class="studio-time">{{ playTime.toFixed(2) }} <span>/ {{ project.duration.toFixed(2) }}s</span></span>
                <span class="studio-transport-divider" aria-hidden="true" />
                <label class="studio-tempo">Tempo<input aria-label="Tempo" type="number" min="40" max="240" step="1" v-model.lazy.number="tempoField" /><span>BPM</span></label>
                <label class="studio-length">Length<input aria-label="Length" type="number" min="0.25" max="30" step="0.25" v-model.lazy.number="durationField" /><span>sec</span></label>
                <label class="studio-check studio-loop"><input type="checkbox" :checked="project.loop" @change="setGlobal('loop', ($event.target as HTMLInputElement).checked)" />Loop</label>
                <div class="studio-add-wrap"><button ref="addButton" class="studio-add-instrument" type="button" :disabled="project.layers.length >= MAX_LAYERS" aria-controls="studio-instrument-picker" :aria-expanded="pickerOpen" @click="pickerOpen = !pickerOpen"><Icon name="plus" :size="16" />Add instrument</button><InstrumentPicker v-if="pickerOpen" :full="project.layers.length >= MAX_LAYERS" @add="addLayer" @close="closePicker()" @drag="dragInstrument" /></div>
                <label class="studio-preview-volume"><Icon name="volume" :size="16" /><span class="sr-only">Preview volume</span><input type="range" min="0" max="1" step="0.05" :value="player.state.volume" @input="player.setVolume(numberValue($event))" /></label>
                <button class="studio-export" type="button" :disabled="!audible || !compiled.document || Boolean(player.state.downloadId)" :aria-busy="Boolean(player.state.downloadId)" @click="download"><Icon name="download" :size="16" /><span>{{ player.state.downloadId ? 'Rendering…' : 'Download WAV' }}</span></button>
            </div>
            <input class="studio-scrub" type="range" aria-label="Playback position" min="0" :max="project.duration" step="0.01" :value="playTime" :disabled="!canSeek" :style="{ '--playback-progress': `${progress * 100}%` }" @input="seek" />
        </section>
        <p v-if="errorText" class="studio-error" role="alert">{{ errorText }}</p>
        <section ref="mixPanel" class="studio-mix" :class="{ 'is-drop-active': dropActive }" aria-label="Instrument layers" @dragover.prevent="dropActive = true" @dragleave.self="dropActive = false" @drop.prevent="drop($event)">
            <div v-if="!project.layers.length" class="studio-empty"><Icon name="layers" :size="28" /><h2>Your sequence starts here</h2><p>Add an instrument, then place a note.</p></div>
            <div v-else class="studio-track-scroll" tabindex="0" aria-label="16-step sequencer. Scroll to see every track.">
                <div class="studio-track-content">
                    <div class="studio-timeline" aria-hidden="true"><span class="studio-track-caption">Tracks <span>{{ project.layers.length }} / {{ MAX_LAYERS }}</span></span><div class="studio-timeline-beats"><div v-for="beat in 4" :key="beat" class="studio-ruler-beat"><div><strong>Beat {{ beat }}</strong><span>{{ ((beat - 1) * 60 / project.tempo).toFixed(2) }}s</span></div><div class="studio-ruler-steps"><span v-for="offset in 4" :key="offset" :class="{ 'is-playhead': playheadStep === (beat - 1) * 4 + offset - 1 }">{{ (beat - 1) * 4 + offset }}</span></div></div></div></div>
                    <article v-for="(layer, index) in project.layers" :key="layer.id" :data-layer="layer.id" class="studio-layer" :class="{ 'is-selected': selectedId === layer.id, 'is-muted': layer.mute }" draggable="true" :aria-label="`${layer.name} layer`" @dragstart="dragLayer($event, layer.id)" @dragover.prevent @drop.stop.prevent="drop($event, layer.id)">
                        <div class="studio-layer-top">
                            <button class="studio-layer-select" type="button" :aria-pressed="selectedId === layer.id" :aria-label="`Edit ${layer.name} layer`" @click="editLayer(layer.id)"><span class="studio-drag-grip" title="Drag to reorder" aria-hidden="true">⠿</span><span><strong>{{ layer.name || instrumentFor(layer.instrument).name }}</strong><small>{{ layer.name.trim().toLowerCase() === instrumentFor(layer.instrument).name.toLowerCase() ? (layer.repeat ? 'Repeats' : 'Once') : instrumentFor(layer.instrument).name }}</small></span></button>
                            <div class="studio-layer-actions"><button type="button" :aria-label="`${layer.mute ? 'Unmute' : 'Mute'} ${layer.name}`" :aria-pressed="layer.mute" @click="muteLayer(layer.id)"><Icon :name="layer.mute ? 'volume-off' : 'volume'" :size="14" /></button><button class="studio-move-up" type="button" :disabled="index === 0" :aria-label="`Move ${layer.name} up`" @click="moveLayer(layer.id, index - 1)"><Icon name="arrow" :size="14" /></button><button class="studio-move-down" type="button" :disabled="index === project.layers.length - 1" :aria-label="`Move ${layer.name} down`" @click="moveLayer(layer.id, index + 1)"><Icon name="arrow" :size="14" /></button><button type="button" :aria-label="`Remove ${layer.name}`" @click="removeLayer(layer.id)"><Icon name="close" :size="14" /></button></div>
                        </div>
                        <div class="studio-steps" role="group" :aria-label="`${layer.name} note pattern`"><div v-for="beat in 4" :key="beat" class="studio-beat"><div class="studio-beat-label"><span>Beat {{ beat }}</span><span>{{ ((beat - 1) * 60 / project.tempo).toFixed(2) }}s</span></div><div class="studio-beat-steps"><button v-for="offset in 4" :key="offset" class="studio-step" type="button" :data-step="(beat - 1) * 4 + offset - 1" :aria-pressed="selectedId === layer.id && selectedStep === (beat - 1) * 4 + offset - 1" :class="{ 'has-note': Boolean(layer.steps[(beat - 1) * 4 + offset - 1]), 'is-current': selectedId === layer.id && selectedStep === (beat - 1) * 4 + offset - 1, 'is-playhead': isPlayhead(layer, (beat - 1) * 4 + offset - 1) }" :aria-label="`${layer.name}, step ${(beat - 1) * 4 + offset}, ${layer.steps[(beat - 1) * 4 + offset - 1] || 'rest'}`" :title="layer.steps[(beat - 1) * 4 + offset - 1] || 'Add a note'" @click="selectStep(layer, (beat - 1) * 4 + offset - 1)" @keydown="stepKeyboard($event, layer, (beat - 1) * 4 + offset - 1)"><span class="studio-step-number">{{ (beat - 1) * 4 + offset }}</span><strong>{{ noteLabel(layer.steps[(beat - 1) * 4 + offset - 1]) }}<small v-if="chordCount(layer.steps[(beat - 1) * 4 + offset - 1])">+{{ chordCount(layer.steps[(beat - 1) * 4 + offset - 1]) }}</small></strong></button></div></div></div>
                        <button class="studio-track-overview" type="button" :aria-label="`Select ${layer.name} pattern`" @click="editLayer(layer.id)"><span class="studio-overview-notes" aria-hidden="true"><span v-for="(note, step) in layer.steps" :key="step" :class="{ 'has-note': Boolean(note), 'is-playhead': isPlayhead(layer, step) }" /></span><span>{{ layer.steps.filter(Boolean).length }} notes</span></button>
                        <div :id="`studio-dock-${layer.id}`" class="studio-mobile-dock" />
                    </article>
                    <div class="studio-canvas-tail" aria-hidden="true"><div /><div class="studio-tail-grid" /></div>
                </div>
            </div>
        </section>
        <Teleport v-if="dockReady" :to="selected ? `#studio-dock-${selected.id}` : 'body'" :disabled="!compactEditor || !selected">
            <section v-if="selected" class="studio-selected-editor" :class="{ 'is-collapsed': dockCollapsed && !compactEditor }" aria-label="Selected track editor">
                <header class="studio-device-header"><label class="studio-layer-name"><span class="sr-only">Layer name</span><input v-model.lazy="layerNameField" maxlength="80" /></label><span v-if="selected.name.trim().toLowerCase() !== instrumentFor(selected.instrument).name.toLowerCase()" class="studio-editor-instrument">{{ instrumentFor(selected.instrument).name }}</span><div class="studio-device-tabs" role="tablist" aria-label="Track controls"><button v-for="(tab, index) in deviceTabs" :id="`studio-tab-${tab.id}`" :key="tab.id" type="button" role="tab" :aria-selected="activeTab === tab.id" :aria-controls="`studio-panel-${tab.id}`" :tabindex="activeTab === tab.id ? 0 : -1" @click="activeTab = tab.id; dockCollapsed = false" @keydown="tabKeyboard($event, index)">{{ tab.label }}</button></div><button v-if="shortWindow && !compactEditor" class="studio-dock-toggle" type="button" :aria-expanded="!dockCollapsed" :aria-label="dockCollapsed ? 'Expand device dock' : 'Collapse device dock'" @click="dockCollapsed = !dockCollapsed"><Icon name="chevron" :size="16" /></button></header>
                <div class="studio-note-edit-row"><label>Step<select aria-label="Step" v-model="selectedStep"><option v-for="number in 16" :key="number" :value="number - 1">{{ number }}</option></select></label><label class="studio-note-field">Note or chord<input v-model.lazy="stepNoteField" :placeholder="instrumentFor(selected.instrument).drum ? 'Kick, Snare, Hat…' : 'C4 E4 G4'" @input="editorError = ''" @keydown.enter.prevent="($event.target as HTMLInputElement).blur()" /></label><label>Note length<select aria-label="Note length" :value="selected.noteLength" @change="setLayer('noteLength', numberValue($event))"><option v-for="length in [1, 2, 4, 8, 16]" :key="length" :value="length">{{ length }} {{ length === 1 ? 'step' : 'steps' }}</option></select></label><button type="button" :disabled="!selected.steps[selectedStep]" @click="clearStep(selected, selectedStep)"><Icon name="close" :size="14" />Clear note</button></div>
                <div v-show="activeTab === 'tone'" id="studio-panel-tone" class="studio-device-panel studio-tone-panel" role="tabpanel" aria-labelledby="studio-tab-tone"><StudioKnob id="studio-volume" label="Volume" :value="selected.gain" :min="0" :max="1" :step="0.01" :scale="100" unit="%" @change="setLayer('gain', $event)" /><StudioKnob v-if="!instrumentFor(selected.instrument).drum" id="studio-pitch" label="Pitch" :value="selected.transpose" :min="-24" :max="24" :step="1" unit=" st" @change="setLayer('transpose', $event)" /><StudioKnob id="studio-pan" label="Pan" :value="selected.pan" :min="-1" :max="1" :step="0.05" bipolar @change="setLayer('pan', $event)" /></div>
                <div v-show="activeTab === 'envelope'" id="studio-panel-envelope" class="studio-device-panel studio-envelope-panel" role="tabpanel" aria-labelledby="studio-tab-envelope"><StudioEnvelope :value="selected.env" /><div class="studio-envelope-faders"><StudioKnob id="studio-attack" label="Attack" :value="selected.env.a" :min="0" :max="2" :step="0.005" :scale="1000" unit=" ms" @change="setEnvelope('a', $event)" /><StudioKnob id="studio-decay" label="Decay" :value="selected.env.d" :min="0" :max="2" :step="0.005" :scale="1000" unit=" ms" @change="setEnvelope('d', $event)" /><StudioKnob id="studio-sustain" label="Sustain" :value="selected.env.s" :min="0" :max="1" :step="0.01" :scale="100" unit="%" @change="setEnvelope('s', $event)" /><StudioKnob id="studio-release" label="Release" :value="selected.env.r" :min="0" :max="2" :step="0.005" :scale="1000" unit=" ms" @change="setEnvelope('r', $event)" /></div></div>
                <div v-show="activeTab === 'effects'" id="studio-panel-effects" class="studio-device-panel studio-effects-panel" role="tabpanel" aria-labelledby="studio-tab-effects"><StudioKnob id="studio-brightness" label="Brightness" :value="selected.cutoff" :min="200" :max="20000" :step="100" unit=" Hz" @change="setLayer('cutoff', $event)" /><StudioKnob id="studio-echo" label="Echo" :value="selected.delay" :min="0" :max="0.75" :step="0.025" :scale="1000" unit=" ms" @change="setLayer('delay', $event)" /></div>
                <div v-show="activeTab === 'pattern'" id="studio-panel-pattern" class="studio-device-panel studio-pattern-panel" role="tabpanel" aria-labelledby="studio-tab-pattern"><label class="studio-check"><input type="checkbox" :checked="selected.repeat" @change="setLayer('repeat', ($event.target as HTMLInputElement).checked)" />Repeat pattern</label><label class="studio-brush-field">Add new steps with<input v-model="brush" placeholder="C4" /></label><button class="studio-clear-pattern" type="button" @click="clearPattern">Clear pattern</button><span>{{ barSeconds.toFixed(2) }}s per pattern</span></div>
            </section>
        </Teleport>
        <p class="sr-only" role="status">{{ status }}</p>
        <dialog ref="utilities" class="studio-utilities" aria-modal="true" aria-labelledby="studio-utilities-title" @close="utilitiesButton?.focus({ preventScroll: true })" @click="utilitiesBackdrop"><header><h2 id="studio-utilities-title">Project utilities</h2><button type="button" aria-label="Close project utilities" @click="closeUtilities"><Icon name="close" :size="18" /></button></header><p>{{ draftState }} · {{ exportState }}</p><label class="studio-seed-field">Seed<input type="number" min="0" max="4294967295" step="1" v-model.lazy.number="seedField" /></label><span class="studio-utility-meta">{{ activeNotes }} active steps · 48 kHz stereo</span><details class="studio-recipe"><summary><Icon name="code" :size="16" />View sound recipe<Icon name="chevron" :size="14" /></summary><pre>{{ compiled.document ? JSON.stringify(compiled.document, null, 2) : compiled.error }}</pre></details></dialog>
        <Teleport v-if="dockReady" to="#studio-instrument-browser"><div class="studio-browser"><label class="studio-browser-search"><Icon name="search" :size="14" /><span class="sr-only">Find studio instrument</span><input v-model="browserSearch" type="search" placeholder="Find instrument" autocomplete="off" /></label><div class="studio-browser-list" role="group" aria-label="Studio instruments"><section v-for="group in browserGroups" :key="group.name"><h3>{{ group.name }}</h3><button v-for="instrument in group.instruments" :key="instrument.id" type="button" :draggable="project.layers.length < MAX_LAYERS" :disabled="project.layers.length >= MAX_LAYERS" :aria-label="`Add ${instrument.name} to sequence`" :title="instrument.detail" @click="addLayer(instrument.id)" @dragstart="dragInstrument($event, instrument.id)"><Icon :name="instrument.drum ? 'impact' : instrument.id === 'noise' ? 'layers' : 'wave'" :size="15" /><span>{{ instrument.name }}</span><Icon class="studio-browser-add" name="plus" :size="12" /></button></section><p v-if="!browserInstruments.length">No instruments found.</p></div><p v-if="project.layers.length >= MAX_LAYERS" class="studio-browser-full">8 tracks used</p></div></Teleport>
    </main>
</template>

<style scoped>
.studio-page { --track-width: 144px; display: flex; flex-direction: column; min-width: 0; width: 100%; max-width: none; height: calc(100dvh - var(--site-header-height, 48px)); margin: 0; padding: 0; background: var(--paper); color: var(--ink); font-size: 13px; line-height: 1.3; overflow: hidden; }
.studio-page :is(button, input, select) { font: inherit; }
.studio-page button { cursor: pointer; }
.studio-page button:disabled { opacity: .4; cursor: default; }
.studio-page :is(input, select) { min-width: 0; color: var(--ink); }
.studio-page :is(input, select, button, summary, [tabindex]):focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
.studio-project-top { display: flex; align-items: center; flex-shrink: 0; gap: 12px; height: 40px; padding: 3px 12px; border-bottom: 1px solid var(--line); background: var(--surface); }
.studio-project-top h1 { min-width: 0; margin: 0; }
.studio-name-field input { width: 200px; max-width: 100%; padding: 3px 0; border: 0; border-bottom: 1px solid transparent; border-radius: 0; background: transparent; font-size: 20px; font-weight: 600; letter-spacing: -.4px; line-height: 1.2; }
.studio-name-field input:hover { border-bottom-color: var(--line); }
.studio-preset-select { max-width: 160px; min-height: 28px; padding: 3px 20px 3px 8px; border: 1px solid var(--line); border-radius: 4px; background: var(--surface-raised); font-size: 12px; }
.studio-draft-state { display: inline-flex; align-items: center; gap: 5px; color: var(--muted); font-size: 12px; white-space: nowrap; }
.studio-draft-state i { width: 5px; height: 5px; border-radius: 50%; background: var(--success); }
.studio-project-actions { display: flex; align-items: center; gap: 3px; margin-left: auto; }
.studio-project-actions button { display: inline-flex; align-items: center; justify-content: center; gap: 6px; min-width: 28px; min-height: 30px; padding: 5px 7px; border-radius: 4px; color: var(--muted-strong); white-space: nowrap; }
.studio-project-actions button:hover { background: var(--surface-hover); color: var(--ink); }
.studio-action-divider { width: 1px; height: 16px; margin-inline: 4px; background: var(--line); }
.studio-transport { position: relative; z-index: 20; display: flex; flex-direction: column; flex-shrink: 0; height: 44px; background: var(--surface); border-bottom: 1px solid var(--line); }
.studio-transport-main { display: flex; align-items: center; gap: 8px; min-height: 40px; padding: 3px 12px; }
.studio-play, .studio-stop, .studio-export, .studio-add-instrument { display: inline-flex; align-items: center; justify-content: center; gap: 6px; min-height: 32px; padding: 6px 10px; border-radius: 4px; font-size: 13px; white-space: nowrap; }
.studio-play { min-width: 70px; background: var(--accent); color: var(--on-accent); font-weight: 600; }
.studio-play:hover { background: var(--accent-hover); }
.studio-stop { width: 28px; padding-inline: 4px; color: var(--muted-strong); }
.studio-stop:hover { background: var(--surface-hover); }
.studio-time { color: var(--ink); font-family: var(--vp-font-family-mono); font-size: 12px; font-variant-numeric: tabular-nums; white-space: nowrap; }
.studio-time > span { color: var(--muted); }
.studio-transport-divider { width: 1px; height: 20px; background: var(--line); }
.studio-tempo, .studio-length { display: inline-flex; align-items: center; gap: 4px; color: var(--muted-strong); font-size: 12px; white-space: nowrap; }
.studio-tempo > span, .studio-length > span { color: var(--muted); }
.studio-transport input[type="number"] { width: 52px; min-height: 28px; padding: 3px 4px; border: 1px solid var(--line); border-radius: 4px; background: var(--surface-raised); font-family: var(--vp-font-family-mono); font-size: 12px; appearance: textfield; }
.studio-transport input[type="number"]::-webkit-inner-spin-button, .studio-transport input[type="number"]::-webkit-outer-spin-button { appearance: none; margin: 0; }
.studio-check { display: inline-flex; align-items: center; gap: 6px; color: var(--muted-strong); white-space: nowrap; font-size: 13px; cursor: pointer; }
.studio-check input { width: 14px; height: 14px; accent-color: var(--accent); }
.studio-add-wrap { position: relative; margin-left: auto; }
.studio-add-instrument { color: var(--muted-strong); }
.studio-add-instrument:hover { color: var(--ink); background: var(--surface-hover); }
.studio-preview-volume { display: flex; align-items: center; gap: 6px; color: var(--muted); }
.studio-preview-volume input { width: 64px; min-height: 28px; accent-color: var(--accent); }
.studio-export { border: 1px solid var(--line); background: var(--surface-raised); color: var(--ink); }
.studio-export:hover { border-color: var(--accent); }
.studio-scrub { display: block; flex-shrink: 0; width: 100%; height: 4px; min-height: 4px; margin: 0; padding: 0; appearance: none; background: transparent; cursor: pointer; }
.studio-scrub::-webkit-slider-runnable-track { height: 3px; background: linear-gradient(to right, var(--accent) 0 var(--playback-progress), var(--line) var(--playback-progress) 100%); }
.studio-scrub::-webkit-slider-thumb { width: 7px; height: 7px; margin-top: -2px; appearance: none; border-radius: 50%; background: var(--accent); }
.studio-scrub::-moz-range-track { height: 3px; background: var(--line); }
.studio-scrub::-moz-range-progress { height: 3px; background: var(--accent); }
.studio-scrub::-moz-range-thumb { width: 7px; height: 7px; border: 0; background: var(--accent); }
.studio-scrub:disabled { opacity: .5; cursor: default; }
.studio-error { flex-shrink: 0; padding: 8px 12px; background: var(--danger-soft); color: var(--danger); font-size: 13px; }
.studio-mix { display: flex; flex-direction: column; flex: 1; min-width: 0; min-height: 0; background: var(--paper); }
.studio-mix.is-drop-active { box-shadow: inset 0 0 0 2px var(--accent); }
.studio-track-scroll { flex: 1; min-height: 0; overflow: auto; scrollbar-width: thin; scrollbar-color: var(--line) var(--paper); }
.studio-track-content { display: flex; flex-direction: column; min-width: calc(var(--track-width) + 576px); min-height: 100%; }
.studio-timeline, .studio-layer { display: grid; grid-template-columns: var(--track-width) minmax(0, 1fr); }
.studio-timeline { position: sticky; top: 0; z-index: 4; height: 32px; flex-shrink: 0; border-bottom: 1px solid var(--line); background: var(--surface); }
.studio-track-caption { display: flex; align-items: center; gap: 7px; padding-left: 12px; border-right: 1px solid var(--line); color: var(--muted-strong); font-size: 12px; }
.studio-track-caption > span { color: var(--muted); }
.studio-timeline-beats, .studio-steps { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); }
.studio-ruler-beat { border-left: 1px solid var(--line); }
.studio-ruler-beat > div:first-child { display: flex; justify-content: space-between; padding: 2px 6px 0; font-size: 12px; line-height: 13px; }
.studio-ruler-beat strong { font-weight: 500; }
.studio-ruler-beat > div:first-child > span { color: var(--muted); font-family: var(--vp-font-family-mono); font-size: 12px; }
.studio-ruler-steps { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); color: var(--muted); font-family: var(--vp-font-family-mono); font-size: 12px; line-height: 15px; text-align: center; }
.studio-ruler-steps .is-playhead { background: var(--ink); color: var(--paper); }
.studio-layer { position: relative; height: 64px; flex-shrink: 0; border-bottom: 1px solid var(--line); background: var(--paper); }
.studio-layer.is-selected { background: color-mix(in srgb, var(--surface-raised) 65%, var(--paper)); }
.studio-layer-top { display: flex; flex-direction: column; justify-content: center; min-width: 0; padding-inline: 8px; border-right: 1px solid var(--line); background: var(--surface); }
.studio-layer.is-selected .studio-layer-top { box-shadow: inset 3px 0 0 var(--accent); background: var(--surface-raised); }
.studio-layer-select { display: flex; align-items: center; gap: 6px; min-width: 0; min-height: 34px; color: var(--ink); text-align: left; }
.studio-drag-grip { color: var(--muted); font-size: 17px; cursor: grab; }
.studio-layer-select strong { display: block; max-width: 106px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; font-weight: 550; line-height: 1.2; }
.studio-layer-select small { display: block; color: var(--muted); font-size: 12px; line-height: 1.2; }
.studio-layer-actions { display: flex; gap: 0; margin-left: 15px; }
.studio-layer-actions button { display: flex; align-items: center; justify-content: center; width: 25px; height: 22px; border-radius: 3px; color: var(--muted); }
.studio-layer-actions button:hover { background: var(--surface-hover); color: var(--ink); }
.studio-layer-actions button[aria-pressed="true"] { color: var(--accent-ink); }
.studio-move-up svg { transform: rotate(-90deg); }
.studio-move-down svg { transform: rotate(90deg); }
.studio-beat { min-width: 0; border-left: 1px solid var(--line); }
.studio-beat-label { display: none; }
.studio-beat-steps { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); height: 100%; }
.studio-step { position: relative; display: flex; align-items: center; justify-content: center; min-width: 36px; width: 100%; height: 100%; padding: 0; border-right: 1px solid color-mix(in srgb, var(--line) 35%, transparent); background: transparent; color: var(--muted); }
.studio-step:hover { background: var(--surface-hover); }
.studio-step-number { display: none; }
.studio-step strong { display: flex; align-items: center; justify-content: center; gap: 1px; width: calc(100% - 8px); height: 30px; padding: 3px 2px; border: 1px solid transparent; border-radius: 4px; font-size: 12px; line-height: 1.2; font-weight: 500; white-space: nowrap; }
.studio-step strong small { font-size: 10px; }
.studio-step.has-note strong { border-color: color-mix(in srgb, var(--accent) 28%, var(--line)); background: var(--accent-soft); color: var(--accent-ink); }
.studio-step.is-current strong { border-color: var(--accent); background: var(--accent); color: var(--on-accent); }
.studio-step.is-current { background: var(--accent-soft); }
.studio-step.is-playhead::after { content: ''; position: absolute; top: 0; bottom: 0; left: 0; width: 2px; background: var(--ink); pointer-events: none; }
.studio-layer.is-muted .studio-steps { opacity: .4; }
.studio-track-overview, .studio-mobile-dock { display: none; }
.studio-canvas-tail { display: grid; flex: 1; min-height: 0; grid-template-columns: var(--track-width) minmax(0, 1fr); }
.studio-canvas-tail > div:first-child { background: var(--surface); border-right: 1px solid var(--line); }
.studio-tail-grid { background-image: linear-gradient(to right, color-mix(in srgb, var(--line) 35%, transparent) 1px, transparent 1px); background-size: calc(100% / 16) 100%; }
.studio-empty { display: flex; align-items: center; justify-content: center; flex-direction: column; gap: 8px; flex: 1; color: var(--muted); }
.studio-empty h2 { font-size: 16px; font-weight: 500; }
.studio-empty p { font-size: 13px; }
.studio-selected-editor { display: flex; flex-direction: column; flex-shrink: 0; height: 200px; border-top: 1px solid var(--line); background: var(--surface-raised); }
.studio-device-header { display: flex; align-items: center; gap: 8px; min-height: 36px; padding: 0 12px; border-bottom: 1px solid var(--line); }
.studio-layer-name { min-width: 0; }
.studio-layer-name input { width: 150px; max-width: 100%; padding: 3px 0; border: 0; background: transparent; font-size: 13px; font-weight: 600; }
.studio-editor-instrument { color: var(--muted); font-size: 12px; }
.studio-device-tabs { display: flex; align-items: stretch; gap: 2px; margin-left: auto; align-self: stretch; }
.studio-device-tabs button { min-width: 64px; padding: 5px 10px; border-bottom: 2px solid transparent; color: var(--muted); font-size: 13px; }
.studio-device-tabs button:hover { color: var(--ink); }
.studio-device-tabs button[aria-selected="true"] { border-bottom-color: var(--accent); color: var(--ink); }
.studio-dock-toggle { display: flex; align-items: center; justify-content: center; width: 30px; height: 30px; color: var(--muted); }
.studio-selected-editor.is-collapsed { height: 36px; }
.studio-selected-editor.is-collapsed :is(.studio-note-edit-row, .studio-device-panel) { display: none !important; }
.studio-selected-editor:not(.is-collapsed) .studio-dock-toggle svg { transform: rotate(180deg); }
.studio-note-edit-row { display: flex; align-items: flex-end; flex-shrink: 0; gap: 10px; min-height: 60px; padding: 7px 12px; }
.studio-note-edit-row > label, .studio-brush-field { display: flex; flex-direction: column; gap: 3px; color: var(--muted); font-size: 12px; }
.studio-note-field { width: 280px; }
.studio-note-edit-row :is(input, select), .studio-brush-field input { width: 100%; min-height: 30px; padding: 4px 7px; border: 1px solid var(--line); border-radius: 4px; background: var(--surface); font-size: 13px; }
.studio-note-field input { font-family: var(--vp-font-family-mono); }
.studio-note-edit-row > button { display: inline-flex; align-items: center; gap: 5px; min-height: 30px; padding: 4px 8px; border: 1px solid var(--line); border-radius: 4px; color: var(--muted-strong); font-size: 12px; white-space: nowrap; }
.studio-note-edit-row > button:hover { background: var(--surface-hover); }
.studio-device-panel { display: grid; align-items: center; gap: 28px; flex: 1; min-height: 0; padding: 8px 12px 12px; overflow: auto; }
.studio-tone-panel { grid-template-columns: repeat(3, minmax(160px, 240px)); }
.studio-effects-panel { grid-template-columns: repeat(2, minmax(160px, 260px)); }
.studio-envelope-panel { grid-template-columns: minmax(200px, 280px) minmax(0, 1fr); gap: 24px; padding-top: 4px; }
.studio-envelope-faders { display: grid; grid-template-columns: repeat(4, minmax(90px, 1fr)); gap: 18px; }
.studio-pattern-panel { display: flex; flex-wrap: wrap; gap: 20px; }
.studio-brush-field { width: 200px; }
.studio-clear-pattern { min-height: 30px; padding: 5px 8px; border: 1px solid var(--line); border-radius: 4px; color: var(--muted-strong); }
.studio-clear-pattern:hover { background: var(--danger-soft); color: var(--danger); }
.studio-pattern-panel > span { margin-left: auto; color: var(--muted); font-size: 12px; }
.studio-utilities { width: min(720px, calc(100% - 32px)); max-height: calc(100dvh - 48px); margin: auto; padding: 20px; overflow: auto; border: 1px solid var(--line); border-radius: 10px; background: var(--surface); color: var(--ink); font: inherit; }
.studio-utilities::backdrop { background: #0009; backdrop-filter: blur(4px); }
.studio-utilities > header { display: flex; align-items: center; justify-content: space-between; gap: 20px; }
.studio-utilities h2 { font-size: 18px; font-weight: 600; }
.studio-utilities > header > button { display: flex; align-items: center; justify-content: center; width: 32px; height: 32px; color: var(--muted); }
.studio-utilities > p { margin-top: 8px; color: var(--muted); font-size: 12px; }
.studio-seed-field { display: flex; align-items: center; gap: 12px; margin-top: 20px; font-size: 13px; }
.studio-seed-field input { width: 180px; min-height: 36px; padding: 6px 8px; border: 1px solid var(--line); border-radius: 4px; background: var(--surface-raised); }
.studio-utility-meta { display: block; margin-block: 15px; color: var(--muted); font-size: 12px; }
.studio-recipe { border-top: 1px solid var(--line); }
.studio-recipe summary { display: flex; align-items: center; gap: 8px; min-height: 40px; color: var(--muted-strong); font-size: 13px; }
.studio-recipe summary > svg:last-child { margin-left: auto; }
.studio-recipe pre { max-height: 45vh; padding: 12px; overflow: auto; border: 1px solid var(--line); border-radius: 4px; background: var(--paper); color: var(--muted-strong); font-family: var(--vp-font-family-mono); font-size: 12px; }
.studio-browser { display: flex; flex-direction: column; gap: 8px; min-height: 0; flex: 1; color: var(--ink); }
.studio-browser-search { display: flex; align-items: center; gap: 6px; min-height: 32px; padding: 4px 7px; border: 1px solid var(--line); border-radius: 4px; background: var(--paper); color: var(--muted); }
.studio-browser-search input { width: 100%; min-width: 0; padding: 0; border: 0; background: transparent; color: var(--ink); font: inherit; font-size: 12px; }
.studio-browser-search input::placeholder { color: var(--muted); }
.studio-browser-search:focus-within { outline: 2px solid var(--focus); outline-offset: 2px; }
.studio-browser-search input:focus { outline: none; }
.studio-browser-list { min-height: 0; overflow-y: auto; overscroll-behavior: contain; scrollbar-width: thin; scrollbar-color: var(--line) transparent; }
.studio-browser-list h3 { padding: 12px 7px 5px; color: var(--muted); font-size: 12px; font-weight: 500; }
.studio-browser-list button { display: flex; align-items: center; gap: 8px; width: 100%; min-height: 30px; padding: 4px 7px; border-radius: 4px; background: transparent; color: var(--muted-strong); font: inherit; font-size: 12px; text-align: left; cursor: grab; }
.studio-browser-list button:hover { background: var(--surface-hover); color: var(--ink); }
.studio-browser-list button:focus-visible { outline: 2px solid var(--focus); outline-offset: -2px; }
.studio-browser-list button:disabled { opacity: .4; cursor: default; }
.studio-browser-list button > span { flex: 1; }
.studio-browser-list button > svg { flex-shrink: 0; }
.studio-browser-add { opacity: 0; color: var(--accent-ink); }
.studio-browser-list button:is(:hover, :focus-visible) .studio-browser-add { opacity: 1; }
.studio-browser-list > p, .studio-browser-full { padding: 4px 7px; color: var(--muted); font-size: 12px; }
.studio-spinner { width: 14px; height: 14px; border: 2px solid color-mix(in srgb, var(--on-accent) 40%, transparent); border-top-color: var(--on-accent); border-radius: 50%; animation: studio-spin .7s linear infinite; }
@keyframes studio-spin { to { transform: rotate(360deg); } }
@media (max-width: 1399px) { .studio-preview-volume { display: none; } }
@media (max-width: 1199px) { .studio-draft-state { display: none; } }
@media (max-width: 1099px) {
    .studio-page { display: block; width: calc(100% - 32px); height: auto; min-height: calc(100dvh - var(--site-header-height, 56px)); margin-inline: auto; padding-block: 12px 24px; overflow: visible; }
    .studio-project-top { flex-wrap: wrap; gap: 8px; height: auto; min-height: 44px; padding: 0 0 10px; border: 0; background: transparent; }
    .studio-project-top h1 { flex: 1; }
    .studio-name-field input { width: 230px; min-height: 44px; }
    .studio-preset-select { min-height: 44px; }
    .studio-project-actions { margin-left: 0; }
    .studio-project-actions button { min-width: 44px; min-height: 44px; }
    .studio-draft-state { display: none; }
    .studio-transport { position: sticky; top: var(--site-header-height, 56px); height: auto; min-height: 48px; }
    .studio-transport-main { flex-wrap: wrap; min-height: 44px; gap: 8px; padding: 8px; }
    .studio-play, .studio-stop, .studio-export, .studio-add-instrument { min-width: 44px; min-height: 44px; }
    .studio-stop { width: 44px; }
    .studio-transport input[type="number"] { min-height: 44px; }
    .studio-scrub { height: 44px; min-height: 44px; }
    .studio-scrub::-webkit-slider-thumb { width: 13px; height: 13px; margin-top: -5px; }
    .studio-scrub::-moz-range-thumb { width: 13px; height: 13px; border-radius: 50%; }
    .studio-check { position: relative; min-width: 44px; min-height: 44px; padding-left: 28px; }
    .studio-check input { position: absolute; inset: 0; z-index: 1; width: 100%; height: 100%; margin: 0; opacity: 0; cursor: pointer; }
    .studio-check::before { content: ''; position: absolute; top: calc(50% - 9px); left: 0; width: 18px; height: 18px; border: 1px solid var(--line); border-radius: 3px; background: var(--surface-raised); }
    .studio-check:has(input:checked)::before { border-color: var(--accent); background: var(--accent); }
    .studio-check:has(input:checked)::after { content: ''; position: absolute; top: calc(50% - 7px); left: 5px; width: 8px; height: 11px; border-right: 2px solid var(--on-accent); border-bottom: 2px solid var(--on-accent); transform: rotate(45deg); }
    .studio-check:focus-within { outline: 2px solid var(--focus); outline-offset: 2px; }
    .studio-preview-volume { display: none; }
    .studio-time { margin-right: auto; }
    .studio-transport-divider { display: none; }
    .studio-mix { margin-top: 12px; }
    .studio-track-scroll { overflow: visible; }
    .studio-track-content { min-width: 0; min-height: 0; }
    .studio-timeline, .studio-canvas-tail { display: none; }
    .studio-layer { display: block; height: auto; padding: 10px 12px; }
    .studio-layer-top { flex-direction: row; align-items: center; justify-content: space-between; padding: 0; border: 0; background: transparent; }
    .studio-layer.is-selected .studio-layer-top { box-shadow: none; background: transparent; }
    .studio-layer.is-selected { border-left: 3px solid var(--accent); padding-left: 9px; }
    .studio-layer-select { min-width: 44px; min-height: 44px; }
    .studio-layer-select strong { max-width: 220px; font-size: 14px; }
    .studio-layer-actions { flex-shrink: 0; margin-left: 0; }
    .studio-layer-actions button { width: 44px; height: 44px; }
    .studio-layer:not(.is-selected) .studio-steps { display: none; }
    .studio-steps { grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; margin-top: 10px; }
    .studio-beat { border: 0; }
    .studio-beat-label { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; color: var(--muted-strong); font-size: 12px; }
    .studio-beat-label > span:last-child { color: var(--muted); font-family: var(--vp-font-family-mono); font-size: 12px; }
    .studio-beat-steps { height: auto; gap: 5px; }
    .studio-step { min-width: 44px; min-height: 48px; border: 1px solid var(--line); border-radius: 4px; }
    .studio-step strong { width: calc(100% - 6px); height: 30px; }
    .studio-step-number { position: absolute; top: 4px; left: 5px; display: block; color: var(--muted); font-family: var(--vp-font-family-mono); font-size: 10px; }
    .studio-step:not(.has-note) .studio-step-number { position: static; font-size: 12px; }
    .studio-step:not(.has-note) strong { display: none; }
    .studio-step.is-playhead::after { top: 0; bottom: 0; left: 0; width: 2px; border-radius: 4px; }
    .studio-track-overview { display: flex; align-items: center; gap: 15px; width: 100%; min-height: 44px; margin-top: 0; color: var(--muted); text-align: left; }
    .studio-overview-notes { display: grid; grid-template-columns: repeat(16, minmax(0, 1fr)); align-items: center; gap: 3px; flex: 1; height: 12px; }
    .studio-overview-notes > span { height: 3px; border-radius: 2px; background: var(--line); }
    .studio-overview-notes > span.has-note { height: 9px; background: var(--accent); }
    .studio-overview-notes > span.is-playhead { background: var(--ink); }
    .studio-track-overview > span:last-child { font-size: 12px; white-space: nowrap; }
    .studio-layer.is-selected .studio-track-overview { display: none; }
    .studio-mobile-dock { display: block; }
    .studio-mobile-dock:empty { display: none; }
    .studio-selected-editor { height: auto; margin-top: 14px; border-top: 1px solid var(--line); background: transparent; }
    .studio-device-header { flex-wrap: wrap; gap: 5px; padding: 6px 0 0; }
    .studio-layer-name { flex: 1; }
    .studio-layer-name input { width: 200px; min-height: 44px; }
    .studio-device-tabs { order: 1; justify-content: space-between; width: 100%; margin-left: 0; }
    .studio-device-tabs button { min-width: 44px; min-height: 44px; }
    .studio-note-edit-row { flex-wrap: wrap; gap: 8px; padding: 10px 0; }
    .studio-note-field { flex: 1; width: auto; min-width: 150px; }
    .studio-note-edit-row :is(input, select), .studio-note-edit-row > button, .studio-brush-field input, .studio-clear-pattern { min-width: 44px; min-height: 44px; }
    .studio-device-panel { padding: 10px 0; overflow: visible; }
    .studio-tone-panel { grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 20px; }
    .studio-envelope-panel { grid-template-columns: minmax(0, 1fr); gap: 10px; }
    .studio-envelope-faders { grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 15px; }
    .studio-effects-panel { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .studio-pattern-panel { gap: 15px; }
    .studio-utilities > header > button { width: 44px; height: 44px; }
    .studio-seed-field input, .studio-recipe summary { min-height: 44px; }
    .studio-empty { min-height: 250px; }
}
@media (max-width: 700px) {
    .studio-project-top h1 { flex-basis: calc(100% - 155px); }
    .studio-name-field input { width: 100%; font-size: 20px; }
    .studio-preset-select { width: 145px; }
    .studio-project-actions { width: 100%; justify-content: space-between; gap: 0; }
    .studio-project-actions button { min-height: 44px; padding-inline: 5px; font-size: 12px; }
    .studio-file-label-detail { display: none; }
    .studio-time > span { display: none; }
    .studio-time { margin-right: 0; }
    .studio-play, .studio-stop, .studio-export, .studio-add-instrument { min-height: 44px; padding-inline: 8px; }
    .studio-transport-main { gap: 4px; padding: 4px; }
    .studio-add-wrap { margin-left: auto; }
    .studio-add-instrument { font-size: 12px; }
    .studio-export { margin-left: auto; }
    .studio-tempo, .studio-length, .studio-loop { order: 1; }
    .studio-tempo, .studio-length { font-size: 12px; }
    .studio-loop { margin-left: auto; }
    .studio-add-wrap { order: 2; width: 100%; margin-left: 0; }
    .studio-add-instrument { width: 100%; border-top: 1px solid var(--line); border-radius: 0; }
    .studio-step { min-height: 48px; }
    .studio-steps { grid-template-columns: minmax(0, 1fr); }
    .studio-layer-top { gap: 4px; }
    .studio-layer-select { flex: 1; }
    .studio-layer-select > span:last-child { flex: 1; min-width: 0; }
    .studio-layer-select strong { max-width: 100%; }
    .studio-layer-actions button { width: 44px; height: 44px; }
    .studio-drag-grip { display: none; }
    .studio-editor-instrument { font-size: 12px; }
    .studio-note-edit-row > label:first-child { width: 52px; }
    .studio-note-field { flex-basis: calc(100% - 60px); min-width: 0; }
    .studio-note-edit-row > label:nth-child(3) { flex: 1; }
    .studio-device-tabs button { flex: 1; min-width: 44px; padding-inline: 4px; font-size: 12px; }
    .studio-note-edit-row :is(input, select), .studio-note-edit-row > button { min-height: 44px; }
    .studio-tone-panel { grid-template-columns: minmax(0, 1fr); gap: 12px; }
    .studio-envelope-faders { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .studio-pattern-panel { align-items: flex-start; }
    .studio-pattern-panel > span { width: 100%; margin-left: 0; }
}
@media (max-width: 360px) { .studio-time { display: none; } .studio-project-actions button { padding-inline: 3px; gap: 4px; } .studio-action-divider { margin-inline: 2px; } .studio-tempo { font-size: 0; } .studio-tempo > span { font-size: 12px; } }
@media (prefers-reduced-motion: reduce) { .studio-spinner { animation-duration: 1.5s; } }
</style>
