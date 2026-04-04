import { onMounted, onUnmounted, ref } from "vue";

function formatRelative(ts) {
  const sec = Math.floor((Date.now() - ts) / 1000);
  if (sec < 5) {
    return "zojuist";
  }
  if (sec < 60) {
    return `${sec}s geleden`;
  }
  const m = Math.floor(sec / 60);
  if (m < 60) {
    return `${m} min geleden`;
  }
  const h = Math.floor(m / 60);
  if (h < 24) {
    return `${h} u geleden`;
  }
  return new Date(ts).toLocaleString("nl-NL", {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Recomputes relative labels when `nowTick` advances (e.g. every 10s). */
export function useRelativeTime(tickMs = 10000) {
  const nowTick = ref(0);
  let timer;

  onMounted(() => {
    timer = setInterval(() => {
      nowTick.value += 1;
    }, tickMs);
  });

  onUnmounted(() => {
    clearInterval(timer);
  });

  function timeLabel(entry) {
    void nowTick.value;
    return formatRelative(entry.archivedAt);
  }

  return { timeLabel };
}
