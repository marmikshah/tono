<script setup lang="ts">
import { withBase } from "vitepress";
import Logo from "./Logo.vue";
</script>

<template>
    <footer class="site-footer">
        <div class="site-container footer-inner">
            <div>
                <a :href="withBase('/')" aria-label="tono home"><Logo /></a>
                <p>Sound, made yours.</p>
            </div>
            <nav aria-label="Footer navigation">
                <a :href="withBase('/get-started/')">Documentation</a
                ><a :href="withBase('/showcase')">Showcase</a
                ><a
                    href="https://github.com/marmikshah/tono"
                    target="_blank"
                    rel="noopener noreferrer"
                    >GitHub</a
                >
            </nav>
            <p class="footer-note">
                Built with Rust.<br /><a
                    href="https://github.com/marmikshah/tono/blob/master/LICENSE"
                    target="_blank"
                    rel="noopener noreferrer"
                    >Open source under MIT.</a
                >
            </p>
        </div>
    </footer>
</template>
