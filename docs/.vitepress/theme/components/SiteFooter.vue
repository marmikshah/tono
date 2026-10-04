<script setup lang="ts">
import { withBase } from "vitepress";
</script>
<template>
    <footer class="workspace-footer"><div class="site-container workspace-footer-inner"><p><strong>tono.</strong> Sound, made yours.</p><nav aria-label="Footer navigation"><a :href="withBase('/get-started/')">Documentation</a><a href="https://github.com/marmikshah/tono" target="_blank" rel="noopener noreferrer">GitHub</a><a href="https://github.com/marmikshah/tono/blob/master/LICENSE" target="_blank" rel="noopener noreferrer">MIT License</a></nav></div></footer>
</template>
<style scoped>
.workspace-footer { margin-top: 6px; border-top: 1px solid var(--line); }
.workspace-footer-inner { display: flex; align-items: center; justify-content: space-between; gap: 20px; min-height: 46px; padding-block: 4px; }
.workspace-footer-inner > p { color: var(--muted); font-size: 10px; }
.workspace-footer-inner strong { margin-right: 9px; color: var(--ink); font-size: 15px; letter-spacing: -.5px; }
.workspace-footer nav { display: flex; gap: 18px; }
.workspace-footer nav a { display: inline-flex; align-items: center; min-height: 44px; color: var(--muted); font-size: 10px; }
.workspace-footer nav a:hover { color: var(--ink); }
@media (max-width: 700px) { .workspace-footer-inner { align-items: flex-start; flex-direction: column; justify-content: center; gap: 3px; padding-block: 17px; } .workspace-footer nav { gap: 21px; } .workspace-footer nav a { min-height: 35px; } }
</style>
