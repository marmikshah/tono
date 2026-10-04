<script setup lang="ts">
withDefaults(defineProps<{ name: string; size?: number }>(), { size: 20 });

const paths: Record<string, string[]> = {
    compass: ["M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Z", "m16 8-3 5-5 3 3-5Z"],
    "chevron-right": ["m9 5 7 7-7 7"],
    sliders: ["M5 3v4", "M5 13v8", "M12 3v11", "M12 20v1", "M19 3v1", "M19 10v11", "M2 7h6v6H2Z", "M9 14h6v6H9Z", "M16 4h6v6h-6Z"],
    book: ["M12 6v15", "M12 6C9 3 5 3 2 4v15c4-1 7-1 10 2 3-3 6-3 10-2V4c-4-1-7-1-10 2Z"],
    cursor: ["m5 3 15 9-8 2-3 7Z"],
    gamepad: ["M7 7h10c3 0 5 8 4 11-1 2-4-1-5-2H8c-1 1-4 4-5 2C2 15 4 7 7 7Z", "M8 10v5", "M5.5 12.5h5", "M16 11h.01", "M18 14h.01"],
    orbit: ["M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6Z", "M3 18C-1 12 13-2 19 3S11 25 5 21", "M4 3c6-3 20 11 17 17S-2 10 4 3Z"],
    arrow: ["M5 12h14", "m13 6 6 6-6 6"],
    external: ["M7 17 17 7", "M7 7h10v10"],
    play: ["m9 5 10 7-10 7Z"],
    pause: ["M8 5v14", "M16 5v14"],
    download: ["M12 3v12", "m7 10 5 5 5-5", "M5 17v4h14v-4"],
    shuffle: [
        "m3 6 3 0 12 12h3",
        "m18 15 3 3-3 3",
        "M3 18h3l4-4",
        "m14 10 4-4h3",
        "m18 3 3 3-3 3",
    ],
    copy: ["M9 9h11v11H9Z", "M15 5V3H3v12h2"],
    check: ["m5 12 4 4L19 6"],
    menu: ["M4 6h16", "M4 12h16", "M4 18h16"],
    close: ["m6 6 12 12", "M6 18 18 6"],
    search: ["M11 3a8 8 0 1 0 0 16 8 8 0 0 0 0-16Z", "m17 17 4 4"],
    plus: ["M12 5v14", "M5 12h14"],
    minus: ["M5 12h14"],
    stop: ["M6 6h12v12H6Z"],
    volume: ["m11 4-6 5H2v6h3l6 5Z", "M15 8a6 6 0 0 1 0 8", "M18 5a10 10 0 0 1 0 14"],
    "volume-off": ["m11 4-6 5H2v6h3l6 5Z", "m16 9 6 6", "m16 15 6-6"],
    star: ["m12 3 2.8 5.7 6.3.9-4.6 4.4 1.1 6.3-5.6-3-5.6 3 1.1-6.3L2.9 9.6l6.3-.9Z"],
    "star-filled": ["m12 3 2.8 5.7 6.3.9-4.6 4.4 1.1 6.3-5.6-3-5.6 3 1.1-6.3L2.9 9.6l6.3-.9Z"],
    list: ["M8 6h13", "M8 12h13", "M8 18h13", "M3 6h.01", "M3 12h.01", "M3 18h.01"],
    grid: ["M3 3h7v7H3Z", "M14 3h7v7h-7Z", "M3 14h7v7H3Z", "M14 14h7v7h-7Z"],
    music: ["M9 18V5l12-2v13", "M9 5v5l12-2", "M9 18a3 3 0 1 1-3-3h3Z", "M21 16a3 3 0 1 1-3-3h3Z"],
    settings: ["M4 6h16", "M4 12h16", "M4 18h16", "M8 3v6", "M16 9v6", "M10 15v6"],
    chevron: ["m6 9 6 6 6-6"],
    undo: ["M3 10h11a6 6 0 0 1 0 12", "m7 5-5 5 5 5"],
    redo: ["M21 10H10a6 6 0 0 0 0 12", "m17 5 5 5-5 5"],
    headphones: ["M3 14v-3a9 9 0 0 1 18 0v3", "M3 12h4v9H3Z", "M17 12h4v9h-4Z"],
    coin: [
        "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20Z",
        "M12 6a6 6 0 1 0 0 12 6 6 0 0 0 0-12Z",
        "M12 9v6",
    ],
    jump: ["M4 18h5", "M7 18V9a5 5 0 0 1 10 0v2", "m14 8 3 3 3-3"],
    laser: ["m15 2-9 11h6l-3 9 10-12h-6Z"],
    explosion: ["m12 2 2 6 6-3-3 6 5 1-6 3 2 6-6-3-5 4 1-7-6-2 6-3-2-6 5 3Z"],
    impact: [
        "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Z",
        "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8Z",
        "M12 1v4",
        "M12 19v4",
    ],
    footstep: [
        "M7 2c-3 0-4 4-3 7l2 6 5-2V6c0-2-1-4-4-4Z",
        "m6 18 5-2 1 3c1 4-5 5-6 2Z",
        "M17 5c3 0 4 4 3 7l-1 3-5-2V9c0-2 1-4 3-4Z",
    ],
    "ui-confirm": ["M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Z", "m8 12 3 3 5-6"],
    "ui-cancel": [
        "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Z",
        "m9 9 6 6",
        "m9 15 6-6",
    ],
    wave: ["M3 10v4", "M7 6v12", "M12 2v20", "M17 6v12", "M21 10v4"],
    layers: ["m12 3 10 5-10 5L2 8Z", "m2 12 10 5 10-5", "m2 16 10 5 10-5"],
    code: ["m8 6-6 6 6 6", "m16 6 6 6-6 6", "m14 3-4 18"],
    github: [
        "M9 19c-5 2-5-3-7-3",
        "M15 22v-4a3.5 3.5 0 0 0-1-3c4 0 7-2 7-6a5 5 0 0 0-1-3c0-1 0-3-1-4 0 0-3 0-4 2a13 13 0 0 0-6 0C8 2 5 2 5 2c-1 1-1 3-1 4a5 5 0 0 0-1 3c0 4 3 6 7 6a3.5 3.5 0 0 0-1 3v4",
    ],
};
</script>

<template>
    <svg
        :width="size"
        :height="size"
        viewBox="0 0 24 24"
        :fill="['play', 'star-filled', 'stop'].includes(name) ? 'currentColor' : 'none'"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
    >
        <path
            v-for="(path, index) in paths[name] || paths.wave"
            :key="index"
            :d="path"
        />
    </svg>
</template>
