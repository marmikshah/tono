<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute } from "vitepress";
import LibraryBrowser from "../components/library/LibraryBrowser.vue";
import { librarySounds } from "../data/library";
import { backgroundMusic } from "../data/bgm";
const route = useRoute();
const kind = ref<"sfx" | "bgm">("sfx");
const ready = ref(false);
function readKind() { kind.value = new URLSearchParams(window.location.search).get("kind") === "bgm" ? "bgm" : "sfx"; }
function changeKind(next: "sfx" | "bgm") {
    if (next === kind.value) return;
    const url = new URL(window.location.href);
    if (next === "bgm") url.searchParams.set("kind", "bgm"); else url.searchParams.delete("kind");
    for (const key of ["q", "category", "saved"]) url.searchParams.delete(key);
    window.history.pushState(window.history.state, "", url);
    kind.value = next;
}
watch(() => route.query, () => { if (typeof window !== "undefined") readKind(); });
onMounted(() => { readKind(); ready.value = true; window.addEventListener("popstate", readKind); });
onBeforeUnmount(() => window.removeEventListener("popstate", readKind));
</script>
<template>
    <LibraryBrowser v-if="ready" :key="kind" :sounds="kind === 'bgm' ? backgroundMusic : librarySounds" :kind="kind" homepage @kind-change="changeKind" />
    <main v-else id="main-content" class="site-container library-loading" role="status">Loading sound browser…</main>
</template>
<style scoped>
.library-loading { padding-block: 25px; color: var(--muted); font-size: 13px; }
</style>
