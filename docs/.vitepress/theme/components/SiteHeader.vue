<script setup lang="ts">
import { ref } from "vue";
import { withBase } from "vitepress";
import Icon from "./Icon.vue";
import Logo from "./Logo.vue";

const menuOpen = ref(false);
const menuButton = ref<HTMLButtonElement>();
function closeMenu() {
    menuOpen.value = false;
    menuButton.value?.focus();
}
</script>

<template>
    <header class="site-header" @keydown.esc="closeMenu">
        <div class="site-container header-inner">
            <a class="brand-link" :href="withBase('/')" aria-label="tono home"
                ><Logo
            /></a>
            <nav
                id="site-navigation"
                class="site-navigation"
                :class="{ 'is-open': menuOpen }"
                aria-label="Main navigation"
            >
                <a :href="withBase('/sounds')" @click="menuOpen = false"
                    >Sound effects</a
                >
                <a :href="withBase('/bgm')" @click="menuOpen = false"
                    >Background music</a
                >
                <a :href="withBase('/create')" @click="menuOpen = false">Sound Studio</a>
                <a :href="withBase('/get-started/')" @click="menuOpen = false"
                    >Documentation <Icon name="external" :size="13"
                /></a>
            </nav>
            <div class="header-actions">
                <a
                    class="icon-link github-link"
                    href="https://github.com/marmikshah/tono"
                    aria-label="tono on GitHub"
                    target="_blank"
                    rel="noopener noreferrer"
                    ><Icon name="github"
                /></a>
                <a
                    class="button button-small button-outline header-cta"
                    :href="withBase('/create')"
                    >Make your own sound <Icon name="arrow" :size="16"
                /></a>
                <button
                    ref="menuButton"
                    class="icon-button menu-trigger"
                    type="button"
                    :aria-expanded="menuOpen"
                    aria-controls="site-navigation"
                    :aria-label="
                        menuOpen ? 'Close navigation' : 'Open navigation'
                    "
                    @click="menuOpen = !menuOpen"
                >
                    <Icon :name="menuOpen ? 'close' : 'menu'" />
                </button>
            </div>
        </div>
    </header>
</template>
