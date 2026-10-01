import { onBeforeUnmount, onMounted, reactive } from "vue";

/** One media element per page keeps previews from playing over each other. */
export function useAudioPlayer() {
  const state = reactive({
    activeId: "",
    playing: false,
    progress: 0,
    error: "",
  });
  let audio: HTMLAudioElement | undefined;
  let request = 0;
  let animation = 0;

  function updateProgress() {
    if (!audio) return;
    state.progress = audio.duration ? audio.currentTime / audio.duration : 0;
    if (!audio.paused && !audio.ended)
      animation = requestAnimationFrame(updateProgress);
  }

  onMounted(() => {
    audio = new Audio();
    audio.preload = "none";
    audio.onplay = () => {
      state.playing = !audio!.paused;
      cancelAnimationFrame(animation);
      updateProgress();
    };
    audio.onpause = () => {
      if (audio?.paused) {
        state.playing = false;
        cancelAnimationFrame(animation);
      }
    };
    audio.onended = () => {
      state.playing = false;
      state.progress = 1;
    };
  });

  function stop() {
    request++;
    audio?.pause();
    if (audio) audio.currentTime = 0;
    cancelAnimationFrame(animation);
    state.playing = false;
    state.progress = 0;
    state.error = "";
  }

  async function toggle(id: string, url: string) {
    if (!audio) return;
    if (state.activeId === id && !audio.paused) {
      request++;
      audio.pause();
      cancelAnimationFrame(animation);
      updateProgress();
      state.playing = false;
      return;
    }
    const current = ++request;
    state.error = "";
    if (state.activeId !== id) {
      audio.pause();
      audio.src = url;
      state.activeId = id;
      state.progress = 0;
    } else if (audio.ended) {
      audio.currentTime = 0;
    }
    try {
      await audio.play();
      if (current === request) state.playing = !audio.paused;
    } catch {
      if (current === request) {
        state.playing = false;
        state.error = "This preview could not play. Please try again.";
      }
    }
  }

  onBeforeUnmount(() => {
    request++;
    cancelAnimationFrame(animation);
    if (audio) {
      audio.pause();
      audio.onplay = audio.onpause = audio.onended = null;
      audio.removeAttribute("src");
      audio.load();
    }
  });

  return { state, toggle, stop };
}

export type AudioPlayer = ReturnType<typeof useAudioPlayer>;
