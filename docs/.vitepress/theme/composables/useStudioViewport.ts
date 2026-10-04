import { computed, onBeforeUnmount, onMounted, ref } from "vue";

export const STUDIO_DESKTOP_QUERY = "(min-width: 1100px) and (hover: hover) and (pointer: fine)";

const desktopMode = ref(false);
let query: MediaQueryList | undefined;
let consumers = 0;

function update() { desktopMode.value = query?.matches ?? false; }

/** One viewport state keeps the page, navigation, and shell in sync. */
export function useStudioViewport() {
  // An asynchronously hydrated page must first match its server placeholder.
  const ready = ref(false);
  const desktop = computed(() => ready.value && desktopMode.value);
  let subscribed = false;
  onMounted(() => {
    subscribed = true;
    if (consumers++ === 0) {
      query = window.matchMedia(STUDIO_DESKTOP_QUERY);
      update();
      query.addEventListener("change", update);
    }
    ready.value = true;
  });
  onBeforeUnmount(() => {
    if (!subscribed) return;
    subscribed = false;
    if (--consumers === 0) {
      query?.removeEventListener("change", update);
      query = undefined;
    }
  });

  return { desktop, ready };
}
