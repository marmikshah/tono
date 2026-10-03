<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { withBase } from "vitepress";
import { useAudioPlayer } from "../composables/useAudio";
import Icon from "../components/Icon.vue";
import StudioKnob from "../studio/StudioKnob.vue";
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
const inspectorPanel = ref<HTMLElement>();
const editorError = ref("");
const status = ref("Your draft saves in this browser.");
const dropActive = ref(false);
const selected = computed(() => project.value.layers.find((layer) => layer.id === selectedId.value));
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
const noteValue = (event: Event) => (event.target as HTMLInputElement).value;
const numberValue = (event: Event) => Number(noteValue(event));

function selectLayer(id: string, resetBrush = false) {
    const changed = id !== selectedId.value;
    selectedId.value = id;
    const layer = project.value.layers.find((layer) => layer.id === id);
    if (changed || resetBrush) brush.value = layer?.steps.find(Boolean) || (layer && instrumentFor(layer.instrument).drum ? instrumentFor(layer.instrument).name.replace("Hi-hat", "Hat") : "C4");
}
async function editLayer(id: string) {
    selectLayer(id);
    if (window.matchMedia("(max-width: 700px)").matches) {
        await nextTick();
        inspectorPanel.value?.focus({ preventScroll: true });
        inspectorPanel.value?.scrollIntoView({ block: "start" });
    }
}
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
function addLayer(instrument: InstrumentId, before?: string) {
    if (project.value.layers.length >= MAX_LAYERS) { editorError.value = `Your mix has room for ${MAX_LAYERS} instruments. Remove a layer to add another.`; return; }
    let number = 1;
    while (project.value.layers.some((layer) => layer.id === `layer_${number}`)) number++;
    const layer = createLayer(instrument, `layer_${number}`);
    commit((project) => { const at = before ? project.layers.findIndex((layer) => layer.id === before) : -1; project.layers.splice(at < 0 ? project.layers.length : at, 0, layer); });
    selectLayer(layer.id);
    selectedStep.value = 0;
}
function removeLayer(id: string) { commit((project) => { project.layers = project.layers.filter((layer) => layer.id !== id); }); }
function muteLayer(id: string) { commit((project) => { const layer = project.layers.find((layer) => layer.id === id); if (layer) layer.mute = !layer.mute; }); }
function moveLayer(id: string, target: number) {
    commit((project) => { const from = project.layers.findIndex((layer) => layer.id === id); if (from < 0 || target < 0 || target >= project.layers.length) return; const [layer] = project.layers.splice(from, 1); project.layers.splice(target, 0, layer); });
}
function toggleStep(layer: StudioLayer, step: number) {
    selectLayer(layer.id);
    selectedStep.value = step;
    const note = brush.value;
    commit((project) => { const current = project.layers.find((entry) => entry.id === layer.id)!; current.steps[step] = current.steps[step] ? "" : note; });
}
function editNotes(text: string) {
    try { parseNotes(text); setLayer("steps", selected.value!.steps.map((note, index) => index === selectedStep.value ? text.trim() : note)); if (text.trim()) brush.value = text.trim(); }
    catch (error) { editorError.value = error instanceof Error ? error.message : "Check this note."; }
}
function clearPattern() { setLayer("steps", Array<string>(16).fill("")); }
function undo() { const previous = undoStack.value.pop(); if (!previous) return; redoStack.value.push(cloneProject(project.value)); project.value = previous; editorError.value = ""; reconcileSelection(true); }
function redo() { const next = redoStack.value.pop(); if (!next) return; undoStack.value.push(cloneProject(project.value)); project.value = next; editorError.value = ""; reconcileSelection(true); }
function starter(id: StarterId) { const next = createStarter(id); commit((project) => { Object.assign(project, next); }); selectedStep.value = 0; reconcileSelection(true); }
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
        selectedStep.value = 0; reconcileSelection(true); status.value = "Project opened. Pick up where you left off.";
    } catch (error) { editorError.value = error instanceof Error ? error.message : "This project could not open."; }
    finally { input.value = ""; }
}
function keyboard(event: KeyboardEvent) {
    if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "z" || ["INPUT", "TEXTAREA", "SELECT"].includes((event.target as HTMLElement).tagName)) return;
    event.preventDefault(); if (event.shiftKey) redo(); else undo();
}
function play() { if (compiled.value.document) void player.toggleDocument(previewId, compiled.value.document); }
function download() { if (compiled.value.document) void player.downloadDocument(previewId, compiled.value.document, `${projectFilename(project.value)}.wav`); }
watch(project, () => {
    player.stop();
    try { localStorage.setItem(DRAFT_KEY, JSON.stringify(project.value)); status.value = "Draft saved in this browser."; } catch { /* A private browser can keep working without persistence. */ }
});
onMounted(() => {
    try {
        const draft = localStorage.getItem(DRAFT_KEY);
        if (draft) { project.value = validateProject(JSON.parse(draft)); reconcileSelection(true); status.value = "Restored your last draft."; }
    } catch { /* Invalid or old drafts never replace the working starter. */ }
});
</script>

<template>
    <main id="main-content" class="site-container studio-page" tabindex="-1" @keydown="keyboard">
        <a class="back-link" :href="withBase('/sounds')"><Icon name="arrow" :size="16" /> Back to the sound library</a>
        <div class="studio-intro">
            <div><p class="eyebrow">THE SOUND STUDIO</p><h1>Make a little <em>noise.</em></h1><p>A few instruments, a few notes, something all your own. Build a sound effect or a little groove, then take it into your game.</p></div>
            <span class="studio-engine-note"><Icon name="wave" :size="17" /> Made here. Played here.</span>
        </div>
        <div class="studio-starters" role="group" aria-label="Starting sounds"><span>Start with a spark</span><button type="button" @click="starter('chime')">Little chime</button><button type="button" @click="starter('zap')">Arcade zap</button><button type="button" @click="starter('beat')">Small beat</button><button type="button" @click="starter('blank')">Blank canvas</button></div>

        <section class="studio-project" aria-label="Your sound">
            <div class="studio-project-top">
                <label class="studio-name-field">Sound name<input :value="project.name" maxlength="80" @change="setGlobal('name', noteValue($event))" /></label>
                <div class="studio-project-actions"><button type="button" aria-label="Undo last edit" :disabled="!undoStack.length" @click="undo"><Icon name="arrow" class="studio-undo-icon" :size="16" /> Undo</button><button type="button" aria-label="Redo last edit" :disabled="!redoStack.length" @click="redo"><Icon name="arrow" :size="16" /> Redo</button><button type="button" @click="fileInput?.click()"><Icon name="layers" :size="16" /> Open project</button><button type="button" @click="saveProject"><Icon name="download" :size="16" /> Save project</button><input ref="fileInput" class="sr-only" type="file" accept="application/json,.json" aria-label="Open studio project JSON" @change="openProject" /></div>
            </div>
            <div class="studio-transport">
                <button class="studio-play" type="button" :disabled="!audible || !compiled.document" :aria-busy="loading" :aria-label="loading ? 'Cancel rendering your sound' : playing ? 'Pause your sound' : 'Play your sound'" @click="play"><span v-if="loading" class="studio-spinner" aria-hidden="true" /><Icon v-else :name="playing ? 'pause' : 'play'" :size="15" />{{ loading ? "Cancel" : playing ? "Pause" : "Play sound" }}</button>
                <label>Tempo <span><input type="number" min="40" max="240" step="1" :value="project.tempo" @change="setGlobal('tempo', numberValue($event))" /> BPM</span></label>
                <label>Length <span><input type="number" min="0.25" max="30" step="0.25" :value="Number(project.duration.toFixed(3))" @change="setGlobal('duration', numberValue($event))" /> sec</span></label>
                <label>Seed <span><input type="number" min="0" max="4294967295" step="1" :value="project.seed" @change="setGlobal('seed', numberValue($event))" /></span></label>
                <label class="studio-loop-control"><input type="checkbox" :checked="project.loop" @change="setGlobal('loop', ($event.target as HTMLInputElement).checked)" /> Loop</label>
                <button class="studio-export" type="button" :disabled="!audible || !compiled.document || Boolean(player.state.downloadId)" :aria-busy="Boolean(player.state.downloadId)" @click="download"><Icon name="download" :size="16" />{{ player.state.downloadId ? "Rendering…" : "Download WAV" }}</button>
            </div>
            <div class="studio-progress" role="progressbar" aria-label="Sound playback" :aria-valuenow="Math.round(progress * 100)" aria-valuemin="0" aria-valuemax="100"><span :style="{ width: `${progress * 100}%` }" /></div>
        </section>
        <p v-if="errorText" class="studio-error" role="alert">{{ errorText }}</p>

        <div class="studio-layout">
            <aside class="studio-palette" aria-label="Instrument palette"><div class="studio-panel-heading"><h2>Instruments</h2><span>{{ project.layers.length }}/{{ MAX_LAYERS }}</span></div><p>Click or drag one into your mix.</p><div class="studio-instrument-grid"><button v-for="instrument in instruments" :key="instrument.id" type="button" draggable="true" :aria-label="`Add ${instrument.name} instrument`" :disabled="project.layers.length >= MAX_LAYERS" @click="addLayer(instrument.id)" @dragstart="dragInstrument($event, instrument.id)"><span class="studio-instrument-icon"><Icon :name="instrument.drum ? 'impact' : instrument.id === 'noise' ? 'layers' : 'wave'" :size="16" /></span><span><strong>{{ instrument.name }}</strong><small>{{ instrument.detail }}</small></span><span class="studio-add-symbol" aria-hidden="true">+</span></button></div></aside>

            <section class="studio-mix" :class="{ 'is-drop-active': dropActive }" aria-label="Instrument layers" @dragover.prevent="dropActive = true" @dragleave.self="dropActive = false" @drop.prevent="drop($event)">
                <div class="studio-panel-heading"><h2>Your mix</h2><span>{{ activeNotes }} active steps</span></div><p class="studio-grid-help">Each row is a 16-step pattern. Click a square to add a note or a rest.</p>
                <div v-if="!project.layers.length" class="studio-empty"><Icon name="layers" :size="34" /><h3>A little room for sound.</h3><p>Drop an instrument here, or choose one from the palette.</p></div>
                <article v-for="(layer, index) in project.layers" :key="layer.id" class="studio-layer" :class="{ 'is-selected': selectedId === layer.id, 'is-muted': layer.mute }" draggable="true" :aria-label="`${layer.name} layer`" @dragstart="dragLayer($event, layer.id)" @dragover.prevent @drop.stop.prevent="drop($event, layer.id)">
                    <div class="studio-layer-top"><button class="studio-layer-select" type="button" :aria-pressed="selectedId === layer.id" :aria-label="`Edit ${layer.name} layer`" title="Edit this layer" @click="editLayer(layer.id)"><span class="studio-drag-grip" title="Drag to reorder" aria-hidden="true">⠿</span><span><strong>{{ layer.name || instrumentFor(layer.instrument).name }}</strong><small>{{ instrumentFor(layer.instrument).name }} <b>·</b> {{ layer.repeat ? 'Repeats' : 'Plays once' }}</small></span></button><div class="studio-layer-actions"><button type="button" :aria-label="`${layer.mute ? 'Unmute' : 'Mute'} ${layer.name}`" :aria-pressed="layer.mute" @click="muteLayer(layer.id)">{{ layer.mute ? "Muted" : "Mute" }}</button><button class="studio-move-up" type="button" :disabled="index === 0" :aria-label="`Move ${layer.name} up`" @click="moveLayer(layer.id, index - 1)"><Icon name="arrow" :size="13" /></button><button class="studio-move-down" type="button" :disabled="index === project.layers.length - 1" :aria-label="`Move ${layer.name} down`" @click="moveLayer(layer.id, index + 1)"><Icon name="arrow" :size="13" /></button><button type="button" :aria-label="`Remove ${layer.name}`" @click="removeLayer(layer.id)"><Icon name="close" :size="14" /></button></div></div>
                    <div class="studio-steps" role="group" :aria-label="`${layer.name} note pattern`"><button v-for="(note, step) in layer.steps" :key="step" type="button" :aria-pressed="Boolean(note)" :class="{ 'is-current': selectedId === layer.id && selectedStep === step, 'is-beat-start': step % 4 === 0 }" :aria-label="`${layer.name}, step ${step + 1}, ${note || 'rest'}`" :title="note || 'Rest'" @click="toggleStep(layer, step)"><span>{{ step + 1 }}</span><strong>{{ note ? note.split(/[\s+,]+/).length > 1 ? "Chord" : note : "·" }}</strong></button></div>
                </article>
                <div v-if="project.layers.length" class="studio-drop-hint"><Icon name="layers" :size="15" /><span>Drop another instrument here. Drag a layer to arrange your mix.</span></div>
                <p class="studio-pattern-length">One pattern: {{ barSeconds.toFixed(2) }} seconds at {{ project.tempo }} BPM.</p>
            </section>

            <aside ref="inspectorPanel" class="studio-inspector" aria-label="Selected instrument controls" tabindex="-1"><template v-if="selected"><div class="studio-panel-heading"><h2>Shape your sound</h2><Icon name="wave" :size="17" /></div><label class="studio-text-field">Layer name<input :value="selected.name" maxlength="80" @change="setLayer('name', noteValue($event))" /></label><div class="studio-knob-stack"><StudioKnob id="studio-volume" label="Volume" :value="selected.gain" :min="0" :max="1" :step="0.01" :scale="100" unit="%" @change="setLayer('gain', $event)" /><StudioKnob v-if="!instrumentFor(selected.instrument).drum" id="studio-pitch" label="Pitch" :value="selected.transpose" :min="-24" :max="24" :step="1" unit=" st" @change="setLayer('transpose', $event)" /><StudioKnob id="studio-pan" label="Pan · left / right" :value="selected.pan" :min="-1" :max="1" :step="0.05" @change="setLayer('pan', $event)" /></div>
                <div class="studio-control-section"><h3>Notes</h3><div class="studio-step-editor"><label for="studio-selected-step">Step<select id="studio-selected-step" v-model="selectedStep"><option v-for="number in 16" :key="number" :value="number - 1">{{ number }}</option></select></label><label for="studio-step-note">Note or chord<input id="studio-step-note" :value="selected.steps[selectedStep]" :placeholder="instrumentFor(selected.instrument).drum ? 'Kick, Snare, Hat…' : 'C4 E4 G4'" @input="editorError = ''" @change="editNotes(noteValue($event))" @keydown.enter.prevent="($event.target as HTMLInputElement).blur()" /></label></div><p>Use note names like C4 or F#3. Separate notes with spaces to make a chord.</p><label class="studio-text-field">Add new steps with<input v-model="brush" placeholder="C4" /></label><div class="studio-note-options"><label>Note length<select :value="selected.noteLength" @change="setLayer('noteLength', numberValue($event))"><option v-for="length in [1, 2, 4, 8, 16]" :key="length" :value="length">{{ length }} {{ length === 1 ? "step" : "steps" }}</option></select></label><label class="studio-repeat"><input type="checkbox" :checked="selected.repeat" @change="setLayer('repeat', ($event.target as HTMLInputElement).checked)" /> Repeat pattern</label></div><button class="studio-clear-pattern" type="button" @click="clearPattern">Clear pattern</button></div>
                <div class="studio-control-section"><h3>Envelope</h3><p>How each note arrives, holds, and fades.</p><div class="studio-knob-stack"><StudioKnob id="studio-attack" label="Attack" :value="selected.env.a" :min="0" :max="2" :step="0.005" :scale="1000" unit=" ms" @change="setEnvelope('a', $event)" /><StudioKnob id="studio-decay" label="Decay" :value="selected.env.d" :min="0" :max="2" :step="0.005" :scale="1000" unit=" ms" @change="setEnvelope('d', $event)" /><StudioKnob id="studio-sustain" label="Sustain" :value="selected.env.s" :min="0" :max="1" :step="0.01" :scale="100" unit="%" @change="setEnvelope('s', $event)" /><StudioKnob id="studio-release" label="Release" :value="selected.env.r" :min="0" :max="2" :step="0.005" :scale="1000" unit=" ms" @change="setEnvelope('r', $event)" /></div></div>
                <div class="studio-control-section"><h3>A little color</h3><div class="studio-knob-stack"><StudioKnob id="studio-brightness" label="Brightness" :value="selected.cutoff" :min="200" :max="20000" :step="100" unit=" Hz" @change="setLayer('cutoff', $event)" /><StudioKnob id="studio-echo" label="Echo" :value="selected.delay" :min="0" :max="0.75" :step="0.025" :scale="1000" unit=" ms" @change="setLayer('delay', $event)" /></div></div>
            </template><div v-else class="studio-inspector-empty"><Icon name="wave" :size="27" /><h2>Every sound has a shape.</h2><p>Add an instrument to choose its notes, volume, and character.</p></div></aside>
        </div>
        <div class="studio-footer-note"><span role="status">{{ status }}</span><span>48 kHz stereo <b>·</b> Editable JSON + WAV</span></div>
        <details class="studio-recipe"><summary><Icon name="code" :size="16" /> View sound recipe</summary><p>The same editable recipe renders in tono. Save your project to keep both the sound and your studio settings.</p><pre>{{ compiled.document ? JSON.stringify(compiled.document, null, 2) : compiled.error }}</pre></details>
    </main>
</template>

<style scoped>
.studio-page { padding-block: 30px 65px; }
.studio-intro { display: flex; align-items: flex-end; justify-content: space-between; gap: 30px; margin-block: 27px 26px; }
.studio-intro h1 { margin-top: 13px; font-size: 54px; font-weight: 500; letter-spacing: -2px; line-height: 1.1; }
.studio-intro p:not(.eyebrow) { max-width: 605px; margin-top: 17px; color: var(--muted); font-size: 12px; line-height: 1.9; }
.studio-engine-note { display: inline-flex; align-items: center; gap: 7px; padding-bottom: 4px; color: #6d805f; font-size: 10px; white-space: nowrap; }
.studio-starters { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; margin-bottom: 23px; }
.studio-starters > span { margin-right: 5px; color: #6c7863; font-size: 10px; }
.studio-starters button { min-height: 35px; padding: 5px 12px; border: 1px solid #dce3d2; border-radius: 5px; background: #f1f3e9; color: #556d49; font-size: 10px; }
.studio-starters button:hover { background: #e7eddd; }
.studio-project { overflow: hidden; border: 1px solid #dce3d1; border-radius: 9px; background: #fffefa; }
.studio-project-top { display: flex; align-items: center; justify-content: space-between; gap: 25px; padding: 18px 22px; border-bottom: 1px solid #e8ebdf; }
.studio-name-field { display: flex; flex-direction: column; gap: 3px; color: #7b8771; font-size: 9px; }
.studio-name-field input { width: 235px; padding: 3px 0; color: #31462f; background: transparent; font-family: inherit; font-size: 19px; font-weight: 500; letter-spacing: -0.5px; }
.studio-project-actions { display: flex; align-items: center; gap: 8px; }
.studio-project-actions button { display: flex; align-items: center; gap: 6px; min-height: 36px; padding: 5px 9px; border-radius: 5px; color: #617456; font-size: 10px; }
.studio-project-actions button:hover { background: #edf2e5; }
.studio-project-actions button:disabled { opacity: 0.35; cursor: default; }
.studio-undo-icon { transform: rotate(180deg); }
.studio-transport { display: flex; align-items: center; flex-wrap: wrap; gap: 21px; padding: 16px 22px; }
.studio-play { display: flex; align-items: center; justify-content: center; gap: 8px; min-width: 112px; min-height: 40px; padding: 8px 15px; border-radius: 5px; background: #385b42; color: #fffefa; font-size: 11px; }
.studio-play:hover { background: #2b4934; }
.studio-transport > label { display: flex; align-items: center; gap: 8px; color: #6d7d62; font-size: 10px; }
.studio-transport > label > span { display: flex; align-items: center; gap: 5px; color: #839077; font-size: 9px; }
.studio-transport input[type="number"] { width: 58px; min-height: 33px; padding: 4px 7px; border: 1px solid #dce3d2; border-radius: 4px; color: #425c3c; background: #f7f8f1; font-family: var(--vp-font-family-mono); font-size: 10px; }
.studio-loop-control { cursor: pointer; }
.studio-loop-control input { accent-color: #5d7e51; }
.studio-export { display: inline-flex; align-items: center; gap: 7px; min-height: 39px; margin-left: auto; padding: 8px 12px; border: 1px solid #d4dec8; border-radius: 5px; color: #3e623b; background: #edf2e5; font-size: 10px; }
.studio-export:hover { background: #e3ebd9; }
.studio-transport button:disabled { opacity: 0.45; cursor: default; }
.studio-progress { height: 3px; background: #e9eddf; }
.studio-progress span { display: block; height: 100%; background: #7f9b69; }
.studio-error { margin-top: 16px; padding: 12px 15px; border: 1px solid #decaba; border-radius: 6px; background: #fbf2e9; color: #8a5038; font-size: 12px; line-height: 1.7; }
.studio-layout { display: grid; grid-template-columns: 180px minmax(0, 1fr) 253px; align-items: start; gap: 18px; margin-top: 23px; }
.studio-panel-heading { display: flex; align-items: center; justify-content: space-between; gap: 9px; color: #6c805e; }
.studio-panel-heading h2 { color: #3f5739; font-size: 14px; font-weight: 550; letter-spacing: -0.2px; }
.studio-panel-heading > span { color: #7a886f; font-size: 9px; }
.studio-palette > p, .studio-grid-help { margin-top: 7px; color: #7a8570; font-size: 9px; line-height: 1.8; }
.studio-instrument-grid { display: flex; flex-direction: column; gap: 5px; margin-top: 16px; }
.studio-instrument-grid > button { display: flex; align-items: center; gap: 9px; min-height: 52px; padding: 9px 9px; border: 1px solid #e0e5d7; border-radius: 6px; background: #fffefa; color: #4e6845; text-align: left; cursor: grab; }
.studio-instrument-grid > button:hover { background: #edf2e5; border-color: #c7d4bb; }
.studio-instrument-grid > button:disabled { opacity: 0.45; cursor: default; }
.studio-instrument-icon { display: flex; align-items: center; color: #789367; }
.studio-instrument-grid strong { display: block; font-size: 10px; font-weight: 550; line-height: 1.5; }
.studio-instrument-grid small { display: block; color: #88927e; font-size: 8px; line-height: 1.6; }
.studio-add-symbol { margin-left: auto; color: #9aad8a; font-size: 17px; }
.studio-mix { min-width: 0; min-height: 285px; padding: 18px; border: 1px solid #dfe5d4; border-radius: 8px; background: #f3f5ec; }
.studio-mix.is-drop-active { border-color: #799467; background: #eaf0df; }
.studio-grid-help { margin-bottom: 17px; }
.studio-layer { margin-top: 12px; padding: 14px 12px 12px; border: 1px solid #dfe5d4; border-radius: 7px; background: #fffefa; cursor: grab; }
.studio-layer.is-selected { border-color: #9db78a; box-shadow: 0 0 0 1px #bccca961; }
.studio-layer.is-muted { opacity: 0.58; }
.studio-layer-top { display: flex; align-items: center; justify-content: space-between; gap: 7px; margin-bottom: 14px; }
.studio-layer-select { display: flex; align-items: center; gap: 8px; min-width: 0; color: #415b39; text-align: left; }
.studio-drag-grip { color: #a0b28f; font-size: 22px; }
.studio-layer-select strong { display: block; max-width: 180px; overflow: hidden; text-overflow: ellipsis; font-size: 11px; font-weight: 550; line-height: 1.5; white-space: nowrap; }
.studio-layer-select small { display: block; color: #7f8a74; font-size: 8px; line-height: 1.7; }
.studio-layer-select b { padding-inline: 3px; font-weight: 400; }
.studio-layer-actions { display: flex; align-items: center; gap: 2px; }
.studio-layer-actions button { display: flex; align-items: center; justify-content: center; min-width: 25px; min-height: 28px; padding: 3px 5px; border-radius: 4px; color: #7c8b71; font-size: 8px; }
.studio-layer-actions button:hover { background: #edf2e5; color: #49643e; }
.studio-layer-actions button:disabled { opacity: 0.3; cursor: default; }
.studio-layer-actions button[aria-pressed="true"] { color: #385b42; background: #edf2e5; }
.studio-move-up svg { transform: rotate(-90deg); }
.studio-move-down svg { transform: rotate(90deg); }
.studio-steps { display: grid; grid-template-columns: repeat(16, minmax(0, 1fr)); gap: 4px; }
.studio-steps button { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 1px; min-width: 0; min-height: 44px; padding: 3px 1px; border: 1px solid #e1e6d8; border-radius: 4px; background: #f2f4eb; color: #a1ac95; }
.studio-steps button span { font-family: var(--vp-font-family-mono); font-size: 7px; line-height: 12px; }
.studio-steps button strong { max-width: 100%; overflow: hidden; font-size: 9px; font-weight: 500; line-height: 15px; text-overflow: ellipsis; }
.studio-steps button[aria-pressed="true"] { border-color: #b7cba3; background: #dfeacd; color: #48623d; }
.studio-steps button.is-beat-start { border-left-color: #becdb0; }
.studio-steps button.is-current { outline: 1px solid #779f61; outline-offset: 2px; }
.studio-drop-hint { display: flex; align-items: center; justify-content: center; gap: 6px; margin-top: 15px; padding: 16px 12px; border: 1px dashed #d0dac4; border-radius: 6px; color: #85967a; font-size: 8px; line-height: 1.7; text-align: center; }
.studio-drop-hint svg { flex-shrink: 0; }
.studio-pattern-length { margin-top: 15px; color: #809273; font-family: var(--vp-font-family-mono); font-size: 8px; line-height: 1.7; }
.studio-empty { padding: 39px 15px; margin-top: 20px; border: 1px dashed #cbd8bd; border-radius: 7px; color: #91a27f; text-align: center; }
.studio-empty h3 { margin-top: 13px; color: #617856; font-size: 15px; font-weight: 500; }
.studio-empty p { margin-top: 7px; font-size: 10px; line-height: 1.8; }
.studio-inspector { min-width: 0; padding: 18px; border: 1px solid #dfe5d4; border-radius: 8px; background: #fffefa; scroll-margin-top: 92px; }
.studio-inspector:focus { outline: none; }
.studio-text-field { display: flex; flex-direction: column; gap: 7px; margin-top: 17px; color: #6d7c62; font-size: 10px; }
.studio-text-field input, .studio-step-editor input { min-width: 0; width: 100%; min-height: 34px; padding: 6px 8px; border: 1px solid #dce3d2; border-radius: 4px; color: #496141; background: #f7f8f1; font-family: inherit; font-size: 10px; }
.studio-knob-stack { display: flex; flex-direction: column; gap: 13px; margin-top: 17px; }
.studio-control-section { margin-top: 21px; padding-top: 19px; border-top: 1px solid #e5e9dc; }
.studio-control-section h3 { color: #536c48; font-size: 11px; font-weight: 550; }
.studio-control-section > p { margin-top: 7px; color: #87927b; font-size: 8px; line-height: 1.8; }
.studio-step-editor { display: grid; grid-template-columns: 55px minmax(0, 1fr); gap: 9px; margin-top: 12px; }
.studio-step-editor label, .studio-note-options > label { display: flex; flex-direction: column; gap: 6px; color: #6e7e63; font-size: 9px; }
.studio-inspector select { width: 100%; min-height: 34px; padding: 5px 7px; border: 1px solid #dce3d2; border-radius: 4px; color: #496141; background-color: #f7f8f1; font-family: inherit; font-size: 10px; }
.studio-note-options { display: flex; align-items: flex-end; justify-content: space-between; gap: 12px; margin-top: 14px; }
.studio-note-options > label.studio-repeat { flex-direction: row; align-items: center; padding-bottom: 7px; cursor: pointer; }
.studio-repeat input { accent-color: #68845b; }
.studio-clear-pattern { margin-top: 10px; min-height: 32px; color: #7a8b6e; font-size: 9px; text-decoration: underline; text-decoration-color: #c1ceb6; text-underline-offset: 4px; }
.studio-inspector-empty { padding-block: 35px; color: #8fa47f; text-align: center; }
.studio-inspector-empty h2 { margin-top: 14px; color: #5d7553; font-size: 16px; font-weight: 500; line-height: 1.5; }
.studio-inspector-empty p { margin-top: 10px; color: #87947b; font-size: 10px; line-height: 1.8; }
.studio-footer-note { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 8px; margin-top: 18px; color: #819072; font-size: 9px; line-height: 1.8; }
.studio-footer-note b { padding-inline: 5px; font-weight: 400; }
.studio-recipe { margin-top: 24px; border: 1px solid #dfe5d4; border-radius: 7px; background: #fffefa; }
.studio-recipe summary { display: flex; align-items: center; gap: 8px; min-height: 45px; padding: 12px 17px; color: #668058; font-size: 11px; cursor: pointer; }
.studio-recipe > p { padding: 0 17px 14px; color: #7d8c70; font-size: 10px; line-height: 1.8; }
.studio-recipe pre { max-height: 360px; margin: 0; padding: 18px; overflow: auto; border-top: 1px solid #e2e8d8; background: #f1f4e9; color: #627851; font-family: var(--vp-font-family-mono); font-size: 9px; line-height: 1.8; }
.studio-page :is(input, select, summary):focus-visible { outline: 2px solid #5d8d66; outline-offset: 3px; border-radius: 4px; }
.studio-spinner { width: 13px; height: 13px; border: 1.5px solid #98b489; border-top-color: white; border-radius: 50%; animation: studio-spin 0.7s linear infinite; }
@keyframes studio-spin { to { transform: rotate(360deg); } }
@media (max-width: 1100px) {
    .studio-layout { grid-template-columns: 150px minmax(0, 1fr) 225px; gap: 12px; }
    .studio-mix { padding: 15px 12px; }
    .studio-layer { padding-inline: 9px; }
    .studio-steps { grid-template-columns: repeat(8, minmax(0, 1fr)); gap: 5px; }
    .studio-inspector { padding: 16px; }
    .studio-transport { gap: 14px; }
    .studio-layer-select strong { max-width: 105px; }
    .studio-layer-actions button { min-width: 22px; padding-inline: 3px; }
}
@media (max-width: 900px) {
    .studio-layout { grid-template-columns: minmax(0, 1fr) 250px; gap: 16px; }
    .studio-palette { grid-column: 1 / -1; }
    .studio-instrument-grid { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 6px; }
    .studio-instrument-grid > button { min-width: 0; }
    .studio-instrument-grid small { display: none; }
    .studio-project-top { flex-wrap: wrap; gap: 10px; }
    .studio-project-actions { gap: 4px; }
    .studio-engine-note { display: none; }
    .studio-layer-select strong { max-width: 160px; }
}
@media (max-width: 700px) {
    .studio-page { padding-block: 23px 43px; }
    .studio-intro { margin-block: 23px; }
    .studio-intro h1 { font-size: 43px; letter-spacing: -1.6px; }
    .studio-intro p:not(.eyebrow) { font-size: 12px; }
    .studio-starters { gap: 6px; }
    .studio-starters > span { width: 100%; margin-bottom: 3px; }
    .studio-starters button { min-height: 38px; padding-inline: 10px; font-size: 10px; }
    .studio-project-top { padding: 16px; }
    .studio-name-field input { width: 100%; }
    .studio-project-actions { width: 100%; justify-content: space-between; gap: 1px; }
    .studio-project-actions button { padding-inline: 3px; min-height: 39px; font-size: 9px; }
    .studio-project-actions button svg { width: 13px; }
    .studio-transport { padding: 14px 16px; gap: 13px; }
    .studio-play { min-width: 107px; }
    .studio-transport > label { gap: 6px; }
    .studio-export { margin-left: 0; }
    .studio-layout { grid-template-columns: minmax(0, 1fr); gap: 19px; }
    .studio-instrument-grid { grid-template-columns: repeat(3, minmax(0, 1fr)); }
    .studio-instrument-grid > button { padding-inline: 8px; gap: 6px; }
    .studio-instrument-icon svg { width: 13px; }
    .studio-instrument-grid strong { font-size: 10px; }
    .studio-add-symbol { font-size: 15px; }
    .studio-mix { padding: 17px 13px; }
    .studio-layer { padding: 13px 10px; }
    .studio-layer-select strong { max-width: 130px; }
    .studio-layer-actions button { min-width: 27px; min-height: 32px; }
    .studio-steps button { min-height: 46px; }
    .studio-steps button strong { font-size: 8px; }
    .studio-steps button span { font-size: 7px; }
    .studio-drop-hint { font-size: 9px; }
    .studio-inspector { padding: 20px; }
    .studio-text-field input, .studio-step-editor input, .studio-inspector select { min-height: 40px; font-size: 12px; }
    .studio-control-section > p { font-size: 10px; }
    .studio-note-options { justify-content: flex-start; gap: 24px; }
    .studio-footer-note { font-size: 9px; }
}
@media (max-width: 370px) {
    .studio-intro h1 { font-size: 39px; }
    .studio-instrument-grid strong { font-size: 9px; }
    .studio-project-actions button { font-size: 8px; gap: 3px; }
    .studio-layer-select { gap: 5px; }
    .studio-layer-select strong { max-width: 94px; }
    .studio-layer-actions button { min-width: 23px; padding-inline: 3px; }
}
@media (prefers-reduced-motion: reduce) { .studio-spinner { animation: none; } }
</style>
