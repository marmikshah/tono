<script setup lang="ts">
import { librarySounds } from "../data/library";
import LibraryBrowser from "../components/library/LibraryBrowser.vue";
</script>

<template>
    <LibraryBrowser :sounds="librarySounds" kind="sfx" />
</template>
