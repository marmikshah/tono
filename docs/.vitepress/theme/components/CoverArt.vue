<script setup lang="ts">
defineProps<{
    kind: "puzzle" | "fantasy" | "arcade" | "jazz" | "piano" | "rhythm";
}>();
</script>

<template>
    <svg viewBox="0 0 360 220" fill="none" aria-hidden="true" class="cover-art">
        <g v-if="kind === 'puzzle'">
            <path fill="#dde4b9" d="M0 0h360v220H0z" />
            <path
                d="M0 55h360M0 110h360M0 165h360M60 0v220M120 0v220M180 0v220M240 0v220M300 0v220"
                stroke="#c9d1a2"
                stroke-width=".7"
            />
            <circle cx="245" cy="76" r="49" fill="#f5f3d8" />
            <g transform="rotate(-14 170 110)">
                <rect
                    x="68"
                    y="53"
                    width="56"
                    height="112"
                    rx="6"
                    fill="#657657"
                />
                <rect
                    x="136"
                    y="78"
                    width="56"
                    height="87"
                    rx="6"
                    fill="#8b9d6b"
                />
                <rect
                    x="204"
                    y="106"
                    width="56"
                    height="59"
                    rx="6"
                    fill="#a8b57b"
                />
                <path
                    d="M79 64h34M147 89h34M215 117h34"
                    stroke="#e1e6ca"
                    stroke-opacity=".45"
                />
            </g>
            <circle cx="288" cy="166" r="7" fill="#b58c59" />
            <circle cx="68" cy="183" r="4" fill="#a2af79" />
        </g>
        <g v-else-if="kind === 'fantasy'">
            <path fill="#e2e8db" d="M0 0h360v220H0z" />
            <circle cx="244" cy="58" r="31" fill="#f8f5df" />
            <path d="M0 106Q66 44 137 109T360 83v137H0z" fill="#b0c2a8" />
            <path d="M0 149Q87 69 173 145T360 110v110H0z" fill="#809b7a" />
            <path d="M0 175Q92 131 178 179T360 151v69H0z" fill="#496f55" />
            <path
                d="M199 220c-6-23 56-26 53-46s-49-11-41-32c-11 24 22 22 23 36s-68 20-67 42z"
                fill="#d4dbb4"
            />
            <path
                d="m49 115 9-28 9 28h-5v12h-8v-12zm260 34 11-36 11 36h-6v16h-9v-16z"
                fill="#476a4f"
            />
            <circle cx="78" cy="57" r="2" fill="#f7f5e7" />
        </g>
        <g v-else-if="kind === 'arcade'">
            <path fill="#29322f" d="M0 0h360v220H0z" />
            <circle cx="181" cy="77" r="48" fill="#d5a48f" />
            <path
                d="M130 85h102M130 97h102M130 110h102M130 121h102"
                stroke="#29322f"
                stroke-width="4"
            />
            <path
                d="m0 145 44-36 36 31 27-17 35 24 47-30 30 18 27-29 62 33 52-31v112H0z"
                fill="#3d5045"
            />
            <path
                d="M0 158h360M0 179h360M0 207h360M180 140 80 220m100-80 100 80m-100-80-35 80m35-80 35 80m-35-80v80"
                stroke="#9abb95"
                stroke-opacity=".5"
                stroke-width=".8"
            />
            <path
                d="m179 153-14 67h30l-14-67"
                fill="#b8c7a7"
                fill-opacity=".25"
            />
            <circle cx="60" cy="47" r="1.5" fill="#dce3cf" />
            <circle cx="282" cy="60" r="1.5" fill="#dce3cf" />
            <circle cx="308" cy="26" r="1" fill="#dce3cf" />
        </g>
        <g v-else-if="kind === 'jazz'">
            <path fill="#d9c9b0" d="M0 0h360v220H0z" />
            <circle
                cx="237"
                cy="105"
                r="76"
                stroke="#b6a68c"
                stroke-width="22"
            />
            <circle
                cx="237"
                cy="105"
                r="46"
                stroke="#bcad94"
                stroke-width="1"
            />
            <g transform="rotate(16 122 109)">
                <rect
                    x="57"
                    y="35"
                    width="23"
                    height="146"
                    rx="11"
                    fill="#546b5a"
                />
                <rect
                    x="96"
                    y="68"
                    width="23"
                    height="113"
                    rx="11"
                    fill="#768269"
                />
                <rect
                    x="135"
                    y="98"
                    width="23"
                    height="83"
                    rx="11"
                    fill="#939775"
                />
                <rect
                    x="174"
                    y="120"
                    width="23"
                    height="61"
                    rx="11"
                    fill="#afb190"
                />
            </g>
        </g>
        <g v-else-if="kind === 'piano'">
            <path fill="#e9e4db" d="M0 0h360v220H0z" />
            <circle cx="279" cy="50" r="63" fill="#ded7cb" />
            <g transform="rotate(-12 180 110)">
                <rect
                    x="50"
                    y="54"
                    width="260"
                    height="120"
                    rx="9"
                    fill="#36463c"
                />
                <rect
                    x="64"
                    y="90"
                    width="232"
                    height="70"
                    rx="3"
                    fill="#f7f4e9"
                />
                <path
                    d="M93 90v70m29-70v70m29-70v70m29-70v70m29-70v70m29-70v70m29-70v70"
                    stroke="#b9b7aa"
                />
                <path
                    d="M87 90h12v42H87zm29 0h12v42h-12zm58 0h12v42h-12zm29 0h12v42h-12zm29 0h12v42h-12z"
                    fill="#36463c"
                />
            </g>
        </g>
        <g v-else>
            <path fill="#e7d8b6" d="M0 0h360v220H0z" />
            <circle cx="222" cy="104" r="79" fill="#c9a86a" />
            <circle
                cx="222"
                cy="104"
                r="57"
                stroke="#eddfbd"
                stroke-width="1"
            />
            <circle
                cx="222"
                cy="104"
                r="34"
                stroke="#eddfbd"
                stroke-width="1"
            />
            <circle cx="222" cy="104" r="11" fill="#ebdcb8" />
            <rect x="62" y="74" width="18" height="100" rx="9" fill="#667557" />
            <rect x="91" y="44" width="18" height="130" rx="9" fill="#7e8a62" />
            <rect
                x="120"
                y="103"
                width="18"
                height="71"
                rx="9"
                fill="#99a478"
            />
        </g>
    </svg>
</template>
