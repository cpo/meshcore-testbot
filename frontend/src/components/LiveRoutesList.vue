<script setup>
import { routeIdUtc } from "../utils/routeFormat.js";

defineProps({
  routes: { type: Array, required: true },
});
</script>

<template>
  <h2>
    <span class="h2-icon live" aria-hidden="true" />
    Live paden
  </h2>
  <ul class="list">
    <li v-for="r in routes.slice().reverse()" :key="r.id" class="live-card">
      <div class="route-head">
        <span
          class="route-swatch"
          :style="{ background: r.routeColor || '#ccc' }"
        />
        <span class="id id--utc" :title="String(r.id)">{{ routeIdUtc(r.id) }}</span>
        <span class="pill pill-live">live</span>
      </div>
      <span class="hops">{{ (r.hops_hex || []).join(" → ") }}</span>
      <span v-if="r.snr != null" class="meta"
        >SNR {{ Number(r.snr).toFixed(1) }}</span
      >
    </li>
    <li v-if="routes.length === 0" class="empty"
      >Wacht op RF-log — nog geen live spoor</li
    >
  </ul>
</template>

<style scoped>
h2 {
  margin: 1.1rem 0 0.45rem;
  font-size: 0.72rem;
  font-weight: 700;
  color: #8b9cb8;
  text-transform: uppercase;
  letter-spacing: 0.09em;
  display: flex;
  align-items: center;
  gap: 0.35rem;
}
.h2-icon {
  width: 8px;
  height: 8px;
  border-radius: 2px;
  flex-shrink: 0;
}
.h2-icon.live {
  background: linear-gradient(135deg, #34d399, #059669);
  box-shadow: 0 0 8px rgba(52, 211, 153, 0.45);
}
.list {
  list-style: none;
  padding: 0;
  margin: 0;
  font-size: 0.8rem;
}
.list li {
  padding: 0.55rem 0;
  border-bottom: 1px solid #252535;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}
.live-card {
  border-radius: 8px;
  padding: 0.5rem 0.45rem !important;
  margin-bottom: 0.35rem;
  background: rgba(52, 211, 153, 0.06);
  border: 1px solid rgba(52, 211, 153, 0.12) !important;
}
.route-head {
  display: flex;
  align-items: center;
  gap: 0.45rem;
  flex-wrap: wrap;
}
.route-swatch {
  width: 10px;
  height: 10px;
  border-radius: 2px;
  flex-shrink: 0;
  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.12);
}
.pill {
  font-size: 0.62rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  padding: 0.12rem 0.35rem;
  border-radius: 999px;
}
.pill-live {
  background: rgba(52, 211, 153, 0.2);
  color: #6ee7b7;
  border: 1px solid rgba(52, 211, 153, 0.35);
}
.list li.empty {
  border: none;
  color: #6b7a90;
  font-style: italic;
}
.id {
  color: #7dd3c0;
  font-variant-numeric: tabular-nums;
  font-size: 0.78rem;
}
.id--utc {
  font-family: ui-monospace, "Cascadia Code", monospace;
  font-size: 0.68rem;
  line-height: 1.35;
  word-break: break-all;
}
.hops {
  word-break: break-all;
  color: #c9cdd6;
  font-size: 0.76rem;
}
.meta {
  font-size: 0.72rem;
  color: #6b7a90;
}
</style>
