<script setup>
defineProps({
  status: { type: String, default: "" },
  livePaused: { type: Boolean, default: false },
  selfPos: { type: Array, default: null },
  routeTtlSec: { type: Number, required: true },
});

defineEmits(["toggle-pause"]);
</script>

<template>
  <header class="brand">
    <span class="brand-mark" aria-hidden="true" />
    <div>
      <h1>MeshCore visor</h1>
      <p class="tagline">Live sporen &amp; spoorboekje</p>
    </div>
  </header>

  <p class="status">{{ status }}</p>
  <div class="live-controls" :class="{ 'live-controls--paused': livePaused }">
    <button
      type="button"
      class="btn-pause"
      :aria-pressed="livePaused"
      @click="$emit('toggle-pause')"
    >
      <span class="btn-pause-icon" aria-hidden="true">{{
        livePaused ? "▶" : "⏸"
      }}</span>
      {{ livePaused ? "Hervat live updates" : "Pauzeer live updates" }}
    </button>
    <p v-if="livePaused" class="pause-hint">
      Nieuwe paden en index-/positie-updates worden niet verwerkt.
    </p>
  </div>
  <p class="hint">
    Live paden vervagen na {{ routeTtlSec }}s — ze belanden in het
    <strong>spoorboekje</strong>. Tik een kaart om het pad weer op de kaart te
    leggen; tik nogmaals om het te verbergen.
  </p>
  <p v-if="selfPos?.length === 2" class="self-pos">
    Zelf (advert): {{ selfPos[0].toFixed(4) }}°, {{ selfPos[1].toFixed(4) }}°
  </p>
</template>

<style scoped>
.brand {
  display: flex;
  align-items: flex-start;
  gap: 0.65rem;
  margin-bottom: 0.75rem;
}
.brand-mark {
  width: 12px;
  height: 12px;
  margin-top: 0.35rem;
  border-radius: 50%;
  background: radial-gradient(circle at 30% 30%, #fbbf24, #d97706);
  box-shadow:
    0 0 12px rgba(251, 191, 36, 0.5),
    inset 0 0 0 2px rgba(0, 0, 0, 0.3);
  flex-shrink: 0;
}
.tagline {
  margin: 0.15rem 0 0;
  font-size: 0.72rem;
  color: #7a8699;
  letter-spacing: 0.02em;
}
h1 {
  margin: 0;
  font-size: 1.2rem;
  font-weight: 700;
  letter-spacing: -0.02em;
}
.status {
  margin: 0 0 0.5rem;
  font-size: 0.82rem;
  color: #9fb0c8;
}
.hint {
  margin: 0 0 1rem;
  font-size: 0.78rem;
  line-height: 1.5;
  color: #7a8699;
}
.hint strong {
  color: #c4b5fd;
  font-weight: 600;
}
.self-pos {
  margin: 0 0 0.75rem;
  font-size: 0.78rem;
  color: #86efac;
}
.live-controls {
  margin: 0 0 0.75rem;
  padding: 0.5rem 0.55rem;
  border-radius: 10px;
  background: rgba(56, 189, 248, 0.06);
  border: 1px solid rgba(56, 189, 248, 0.15);
}
.live-controls--paused {
  background: rgba(251, 191, 36, 0.08);
  border-color: rgba(251, 191, 36, 0.22);
}
.btn-pause {
  display: inline-flex;
  align-items: center;
  gap: 0.45rem;
  width: 100%;
  justify-content: center;
  font: inherit;
  font-size: 0.78rem;
  font-weight: 600;
  padding: 0.45rem 0.65rem;
  border-radius: 8px;
  border: 1px solid rgba(125, 211, 252, 0.35);
  background: linear-gradient(
    180deg,
    rgba(56, 189, 248, 0.15),
    rgba(14, 165, 233, 0.08)
  );
  color: #7dd3f8;
  cursor: pointer;
}
.btn-pause[aria-pressed="true"] {
  border-color: rgba(251, 191, 36, 0.45);
  background: linear-gradient(
    180deg,
    rgba(251, 191, 36, 0.15),
    rgba(217, 119, 6, 0.1)
  );
  color: #fcd34d;
}
.btn-pause-icon {
  font-size: 0.85rem;
  opacity: 0.95;
}
.pause-hint {
  margin: 0.45rem 0 0;
  font-size: 0.68rem;
  line-height: 1.45;
  color: #a89f88;
}
</style>
