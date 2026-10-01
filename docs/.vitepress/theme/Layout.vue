<script setup lang="ts">
import { Content, useData } from "vitepress";
import DefaultTheme from "vitepress/theme";
import SiteHeader from "./components/SiteHeader.vue";
import SiteFooter from "./components/SiteFooter.vue";

const { frontmatter } = useData();
</script>

<template>
    <div
        v-if="['landing', 'library'].includes(frontmatter.layout)"
        class="tono-site"
    >
        <a class="skip-link" href="#main-content">Skip to content</a>
        <SiteHeader />
        <Content />
        <SiteFooter />
    </div>
    <DefaultTheme.Layout v-else />
</template>
