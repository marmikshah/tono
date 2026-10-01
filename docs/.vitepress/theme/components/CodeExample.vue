<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from "vue";
import Icon from "./Icon.vue";

const snippets = {
    CLI: `# From a tono checkout\ncargo run --locked -- templates\n\n# Shape your next laser sound\ncargo run --locked -- generate laser \\\n  --seed 42 --brightness 0.7 --punch 0.8 \\\n  -o target/my-game`,
    Rust: `use tono_core::generate::{\n    generate_sfx, SfxSpec, SfxTemplate,\n};\n\nlet spec = SfxSpec::new(SfxTemplate::Laser, 42);\nlet sound = generate_sfx(&spec)?;\nlet samples = tono_core::render::render(&sound);`,
    Python: `import tono\n\nsong = tono.Song("first-groove", tempo=112, seed=42)\nbass = song.track("bass", tono.instruments.bass("finger"))\nnotes = tono.Pattern()\nnotes.notes(["C2", "E2", "G2", "C3"], durations=1)\nsong.arrange(bass, notes, bars=range(4))\naudio = song.compile(sample_rate=48000).render()`,
};
type Language = keyof typeof snippets;
const selected = ref<Language>("CLI");
const languages = Object.keys(snippets) as Language[];
const snippet = computed(() => snippets[selected.value]);
const copied = ref(false);
const message = ref("");
let timer: ReturnType<typeof setTimeout> | undefined;
async function copy() {
    try {
        await navigator.clipboard.writeText(snippet.value);
        copied.value = true;
        message.value = "Code copied to clipboard.";
        clearTimeout(timer);
        timer = setTimeout(() => {
            copied.value = false;
            message.value = "";
        }, 2000);
    } catch {
        message.value = "Copy is unavailable. Select the code to copy it.";
    }
}
onBeforeUnmount(() => clearTimeout(timer));
</script>

<template>
    <div class="code-example">
        <div class="code-toolbar">
            <div class="code-tabs" role="group" aria-label="Code language">
                <button
                    v-for="language in languages"
                    :key="language"
                    :aria-pressed="selected === language"
                    type="button"
                    @click="
                        selected = language;
                        copied = false;
                        message = '';
                    "
                >
                    {{ language }}
                </button>
            </div>
            <button
                class="copy-code"
                type="button"
                @click="copy"
                :aria-label="`Copy ${selected} example`"
            >
                <Icon :name="copied ? 'check' : 'copy'" :size="15" />{{
                    copied ? "Copied" : "Copy"
                }}
            </button>
        </div>
        <pre><code><span v-for="(line, index) in snippet.split('\n')" :key="index" class="code-line"><span class="line-number" aria-hidden="true">{{ index + 1 }}</span><span :class="{ 'code-comment': line.startsWith('#') }">{{ line || ' ' }}</span></span></code></pre>
        <p class="code-caption">
            {{
                selected === "Python"
                    ? "Compose with the typed Python song API."
                    : "Same recipe + same seed = the same sound."
            }}
        </p>
        <p
            class="copy-message"
            :class="{ 'sr-only': copied || !message }"
            aria-live="polite"
        >
            {{ message }}
        </p>
    </div>
</template>
