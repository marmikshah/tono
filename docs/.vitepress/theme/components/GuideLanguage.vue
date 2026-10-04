<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { onContentUpdated } from "vitepress";

type Language = "Rust" | "Python";
const preferenceKey = "tono-guide-language-v1";
const selected = ref<Language>("Rust");
let mounted = false;

function syncExamples() {
    if (!mounted) return;
    for (const group of Array.from(document.querySelectorAll<HTMLElement>(".VPDoc .vp-code-group"))) {
        const tabs = group.querySelector<HTMLElement>(":scope > .tabs");
        const inputs = Array.from(tabs?.querySelectorAll<HTMLInputElement>("input") ?? []);
        const labels = inputs.map((input) => input.nextElementSibling?.textContent?.trim());
        if (!labels.includes("Rust") || !labels.includes("Python")) continue;
        const index = labels.indexOf(selected.value);
        const blocks = group.querySelector<HTMLElement>(":scope > .blocks");
        const next = blocks?.children[index];
        if (!next) continue;
        tabs?.setAttribute("role", "radiogroup");
        tabs?.setAttribute("aria-label", "Example language");
        inputs.forEach((input, at) => { input.checked = at === index; });
        const changed = !next.classList.contains("active");
        Array.from(blocks!.children).forEach((block, at) => block.classList.toggle("active", at === index));
        if (changed) window.dispatchEvent(new CustomEvent("vitepress:codeGroupTabActivate", { detail: next }));
    }
}

function choose(language: Language) {
    selected.value = language;
    try { localStorage.setItem(preferenceKey, language); } catch { /* Browsing works without storage. */ }
    syncExamples();
}

function groupChanged(event: Event) {
    const input = event.target;
    if (!(input instanceof HTMLInputElement) || !input.matches(".VPDoc .vp-code-group input")) return;
    const language = input.nextElementSibling?.textContent?.trim();
    if (language === "Rust" || language === "Python") choose(language);
}

onContentUpdated(() => { if (mounted) void nextTick(syncExamples); });
onMounted(() => {
    mounted = true;
    try {
        const preference = localStorage.getItem(preferenceKey);
        if (preference === "Rust" || preference === "Python") selected.value = preference;
    } catch { /* Use Rust when storage is unavailable. */ }
    document.addEventListener("change", groupChanged);
    void nextTick(syncExamples);
});
onBeforeUnmount(() => { mounted = false; document.removeEventListener("change", groupChanged); });
</script>

<template>
    <div class="guide-language">
        <span>Examples in</span>
        <div role="group" aria-label="Guide language">
            <button v-for="language in (['Rust', 'Python'] as const)" :key="language" type="button" :aria-pressed="selected === language" @click="choose(language)">{{ language }}</button>
        </div>
    </div>
</template>

<style scoped>
.guide-language { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-bottom: 22px; padding-bottom: 16px; border-bottom: 1px solid var(--line); }
.guide-language > span { color: var(--muted); font-size: 13px; }
.guide-language > div { display: flex; gap: 3px; padding: 3px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface); }
.guide-language button { min-width: 74px; min-height: 38px; padding: 0 14px; border-radius: 5px; color: var(--muted-strong); font: inherit; font-size: 13px; cursor: pointer; }
.guide-language button:hover { background: var(--surface-hover); }
.guide-language button[aria-pressed="true"] { background: var(--accent); color: var(--on-accent); font-weight: 600; }
.guide-language button:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
@media (max-width: 1099px), (pointer: coarse) { .guide-language button { min-height: 44px; } }
</style>
