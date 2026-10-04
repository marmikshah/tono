<script setup lang="ts">
import { backgroundMusic } from "../data/bgm";
import LibraryBrowser from "../components/library/LibraryBrowser.vue";
</script>

<template>
    <LibraryBrowser :sounds="backgroundMusic" kind="bgm" />
</template>
