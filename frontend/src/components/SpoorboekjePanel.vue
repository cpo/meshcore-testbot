<script setup>
import { computed } from "vue";
import { useRelativeTime } from "../composables/useRelativeTime.js";
import { routeHopNodes, routeIdUtc } from "../utils/routeFormat.js";

const expanded = defineModel("expanded", { type: Boolean, default: false });

const props = defineProps({
  routeHistoryCount: { type: Number, default: 0 },
  selectedHistoryIds: { type: Array, default: () => [] },
  spoorboekjePage: { type: Number, default: 1 },
  spoorboekjeTotalPages: { type: Number, default: 0 },
  historyPageSlice: { type: Array, default: () => [] },
  pageSize: { type: Number, required: true },
});

defineEmits([
  "clear-overlays",
  "clear-spoorboekje",
  "fit-all",
  "prev-page",
  "next-page",
  "toggle-history",
]);

const { timeLabel } = useRelativeTime();

const selectedCount = computed(() => props.selectedHistoryIds.length);

function isSelected(id) {
  return props.selectedHistoryIds.includes(String(id));
}

</script>

<template>
  <div class="archive-block">
    <div class="archive-toolbar">
      <button
        type="button"
        class="archive-toggle"
        :aria-expanded="expanded"
        aria-controls="archive-panel"
        id="archive-heading"
        @click="expanded = !expanded"
      >
        <span class="h2-icon archive" aria-hidden="true" />
        <span class="archive-toggle-title">Spoorboekje</span>
        <span class="archive-count">{{ routeHistoryCount }}</span>
        <span
          v-if="selectedCount > 0"
          class="archive-selected-badge"
          :title="selectedCount + ' pad(en) op de kaart'"
          >{{ selectedCount }}×</span
        >
        <span class="archive-chevron" aria-hidden="true">{{
          expanded ? "▾" : "▸"
        }}</span>
      </button>
      <button
        v-if="selectedCount > 0"
        type="button"
        class="btn-ghost btn-ghost--sm"
        @click.stop="$emit('clear-overlays')"
      >
        Verberg
      </button>
    </div>
    <div
      v-show="expanded"
      id="archive-panel"
      class="archive-panel"
      role="region"
      aria-labelledby="archive-heading"
    >
      <p class="archive-blurb">
        Opgeslagen routes (lokaal in deze browser) — gestippeld op de kaart.
        Tik om te tonen of te verbergen.
      </p>
      <div
        v-if="routeHistoryCount > 0 && spoorboekjeTotalPages > 0"
        class="archive-pagination"
        role="navigation"
        aria-label="Spoorboekje paginering"
      >
        <button
          type="button"
          class="btn-ghost btn-ghost--sm btn-page"
          :disabled="spoorboekjePage <= 1"
          @click="$emit('prev-page')"
        >
          Vorige
        </button>
        <span class="archive-pagination-label">
          Pagina {{ spoorboekjePage }} van {{ spoorboekjeTotalPages }}
          <span class="archive-pagination-range">
            ({{ (spoorboekjePage - 1) * pageSize + 1 }}–{{
              Math.min(spoorboekjePage * pageSize, routeHistoryCount)
            }}
            van {{ routeHistoryCount }})
          </span>
        </span>
        <button
          type="button"
          class="btn-ghost btn-ghost--sm btn-page"
          :disabled="spoorboekjePage >= spoorboekjeTotalPages"
          @click="$emit('next-page')"
        >
          Volgende
        </button>
      </div>
      <div v-if="routeHistoryCount > 0" class="archive-actions">
        <button
          type="button"
          class="btn-clear-archive"
          @click="$emit('clear-spoorboekje')"
        >
          Spoorboekje legen
        </button>
      </div>

      <TransitionGroup name="hist" tag="ul" class="history-rail">
        <li
          v-for="entry in historyPageSlice"
          :key="entry.id + '-' + entry.archivedAt"
          class="history-card"
          :class="{ 'history-card--on': isSelected(entry.id) }"
        >
          <button
            type="button"
            class="history-card-btn"
            :aria-pressed="isSelected(entry.id)"
            @click="$emit('toggle-history', entry)"
          >
            <span class="history-rail-line" aria-hidden="true" />
            <span
              class="history-swatch"
              :style="{ background: entry.routeColor || '#64748b' }"
            />
            <div class="history-body">
              <div class="history-top">
                <span class="id id--utc" :title="String(entry.id)">{{
                  routeIdUtc(entry.id)
                }}</span>
                <span class="history-time">{{ timeLabel(entry) }}</span>
              </div>
              <span class="hops mini">{{
                (entry.hops_hex || []).slice(0, 8).join(" → ")
              }}{{
                (entry.hops_hex || []).length > 8 ? " → …" : ""
              }}</span>
              <ol
                v-if="routeHopNodes(entry).length"
                class="route-nodes route-nodes--compact"
              >
                <li
                  v-for="node in routeHopNodes(entry)"
                  :key="`${entry.id}-hop-${node.n}-${entry.archivedAt}`"
                >
                  <span class="route-node-badge" aria-hidden="true">{{
                    node.n
                  }}</span>
                  <span class="route-node-name">{{ node.name }}</span>
                  <span v-if="node.hopHex" class="route-node-hex">{{
                    node.hopHex
                  }}</span>
                </li>
              </ol>
              <span v-if="entry.snr != null" class="meta"
                >SNR {{ Number(entry.snr).toFixed(1) }}</span
              >
            </div>
            <span class="history-chevron">{{
              isSelected(entry.id) ? "✓" : "+"
            }}</span>
          </button>
        </li>
      </TransitionGroup>
      <p v-if="routeHistoryCount === 0" class="empty archive-empty">
        Nog geen archief — live paden komen hier automatisch terecht.
      </p>

      <button
        v-if="routeHistoryCount > 0 && selectedCount > 0"
        type="button"
        class="btn-fit"
        @click="$emit('fit-all')"
      >
        Zoom: alle zichtbare paden
      </button>
    </div>
  </div>
</template>

<style scoped>
.h2-icon {
  width: 8px;
  height: 8px;
  border-radius: 2px;
  flex-shrink: 0;
}
.h2-icon.archive {
  background: linear-gradient(135deg, #a78bfa, #6366f1);
  box-shadow: 0 0 8px rgba(167, 139, 250, 0.35);
}
.archive-block {
  margin-top: 1rem;
}
.archive-toolbar {
  display: flex;
  align-items: stretch;
  gap: 0.35rem;
}
.archive-toggle {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 0.4rem;
  font: inherit;
  padding: 0.45rem 0.4rem;
  border-radius: 8px;
  border: 1px solid #2e2a40;
  background: rgba(139, 92, 246, 0.08);
  color: #e8e8ef;
  cursor: pointer;
  text-align: left;
}
.archive-toggle:hover {
  background: rgba(139, 92, 246, 0.14);
  border-color: #4c3f6b;
}
.archive-toggle-title {
  flex: 1;
  font-size: 0.72rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.09em;
  color: #8b9cb8;
}
.archive-count {
  font-size: 0.68rem;
  font-weight: 600;
  color: #9fb0c8;
  background: rgba(255, 255, 255, 0.06);
  padding: 0.1rem 0.4rem;
  border-radius: 999px;
}
.archive-selected-badge {
  font-size: 0.62rem;
  font-weight: 600;
  color: #c4b5fd;
  background: rgba(167, 139, 250, 0.12);
  padding: 0.1rem 0.35rem;
  border-radius: 999px;
  border: 1px solid rgba(167, 139, 250, 0.35);
}
.archive-chevron {
  color: #7a8699;
  font-size: 0.75rem;
}
.archive-panel {
  margin-top: 0.35rem;
  padding-left: 0.45rem;
  border-left: 2px solid rgba(167, 139, 250, 0.35);
}
.archive-blurb {
  margin: 0 0 0.65rem;
  font-size: 0.72rem;
  line-height: 1.45;
  color: #6b7a90;
}
.archive-pagination {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.4rem;
  margin: 0 0 0.65rem;
  flex-wrap: wrap;
}
.archive-pagination-label {
  flex: 1;
  min-width: 8rem;
  text-align: center;
  font-size: 0.7rem;
  font-weight: 600;
  color: #9fb0c8;
  line-height: 1.35;
}
.archive-pagination-range {
  display: block;
  font-weight: 500;
  font-size: 0.65rem;
  color: #6b7a90;
  margin-top: 0.15rem;
}
.btn-page:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}
.archive-actions {
  margin: 0 0 0.65rem;
}
.btn-clear-archive {
  font: inherit;
  font-size: 0.72rem;
  font-weight: 600;
  padding: 0.35rem 0.65rem;
  border-radius: 6px;
  border: 1px solid rgba(248, 113, 113, 0.45);
  background: rgba(248, 113, 113, 0.08);
  color: #fca5a5;
  cursor: pointer;
}
.btn-clear-archive:hover {
  background: rgba(248, 113, 113, 0.16);
  border-color: rgba(248, 113, 113, 0.65);
  color: #fecaca;
}
.btn-ghost {
  font: inherit;
  font-size: 0.68rem;
  font-weight: 600;
  padding: 0.3rem 0.55rem;
  border-radius: 6px;
  border: 1px solid #3f3f55;
  background: rgba(255, 255, 255, 0.04);
  color: #c4c9d4;
  cursor: pointer;
}
.btn-ghost:hover {
  background: rgba(167, 139, 250, 0.12);
  border-color: #7c6bbd;
  color: #e8e4ff;
}
.btn-ghost--sm {
  flex-shrink: 0;
  align-self: flex-start;
  padding: 0.42rem 0.5rem;
  white-space: nowrap;
}
.btn-fit {
  width: 100%;
  margin: 0.75rem 0 0;
  font: inherit;
  font-size: 0.78rem;
  font-weight: 600;
  padding: 0.5rem 0.75rem;
  border-radius: 8px;
  border: 1px solid rgba(125, 211, 252, 0.35);
  background: linear-gradient(180deg, rgba(56, 189, 248, 0.15), rgba(14, 116, 144, 0.12));
  color: #7dd3f8;
  cursor: pointer;
}
.btn-fit:hover {
  filter: brightness(1.08);
}

.history-rail {
  list-style: none;
  padding: 0;
  margin: 0;
  position: relative;
}
.history-card {
  margin-bottom: 0.4rem;
}
.history-card-btn {
  width: 100%;
  display: flex;
  align-items: stretch;
  gap: 0;
  text-align: left;
  font: inherit;
  color: inherit;
  cursor: pointer;
  padding: 0;
  border: none;
  background: transparent;
  border-radius: 10px;
  position: relative;
  overflow: hidden;
  transition:
    transform 0.15s ease,
    box-shadow 0.2s ease;
}
.history-card-btn:hover {
  transform: translateX(2px);
}
.history-card--on .history-card-btn {
  box-shadow:
    0 0 0 2px rgba(167, 139, 250, 0.55),
    0 6px 20px rgba(99, 102, 241, 0.15);
}
.history-rail-line {
  width: 3px;
  background: linear-gradient(180deg, #6366f1, #a78bfa 50%, #4c1d95);
  border-radius: 2px;
  flex-shrink: 0;
  opacity: 0.85;
}
.history-swatch {
  width: 6px;
  align-self: stretch;
  min-height: 100%;
  flex-shrink: 0;
}
.history-body {
  flex: 1;
  padding: 0.55rem 0.6rem 0.55rem 0.5rem;
  background: rgba(30, 27, 46, 0.65);
  border: 1px solid #2e2a40;
  border-left: none;
  border-radius: 0 10px 10px 0;
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
}
.history-card--on .history-body {
  background: rgba(67, 56, 102, 0.45);
  border-color: rgba(167, 139, 250, 0.35);
}
.history-top {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  gap: 0.35rem;
}
.history-time {
  font-size: 0.65rem;
  color: #8b9aaf;
  white-space: nowrap;
}
.history-chevron {
  width: 2rem;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 1rem;
  font-weight: 700;
  color: #a78bfa;
  background: rgba(99, 102, 241, 0.12);
  border: 1px solid #2e2a40;
  border-left: none;
  border-radius: 0 10px 10px 0;
}
.history-card--on .history-chevron {
  background: rgba(52, 211, 153, 0.15);
  color: #6ee7b7;
}
.archive-empty {
  margin-top: 0.25rem !important;
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
.hops.mini {
  font-size: 0.7rem;
  opacity: 0.92;
}
.route-nodes {
  list-style: none;
  padding: 0.35rem 0 0;
  margin: 0.25rem 0 0;
  border-top: 1px solid rgba(255, 255, 255, 0.06);
  display: flex;
  flex-direction: column;
  gap: 0.28rem;
}
.route-nodes--compact {
  padding-top: 0.28rem;
  margin-top: 0.2rem;
  gap: 0.2rem;
}
.route-nodes li {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 0.35rem 0.45rem;
  padding: 0;
  border: none;
  font-size: 0.74rem;
  line-height: 1.35;
}
.route-node-badge {
  flex-shrink: 0;
  min-width: 1.1rem;
  height: 1.1rem;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 0.62rem;
  font-weight: 700;
  color: #93c5fd;
  background: rgba(59, 130, 246, 0.22);
  border-radius: 4px;
}
.route-node-name {
  color: #e2e6ef;
  font-weight: 500;
}
.route-node-hex {
  font-size: 0.65rem;
  color: #6b7a90;
  font-family: ui-monospace, monospace;
}
.meta {
  font-size: 0.72rem;
  color: #6b7a90;
}
.list li.empty {
  border: none;
  color: #6b7a90;
  font-style: italic;
}

.hist-move,
.hist-enter-active,
.hist-leave-active {
  transition: all 0.35s ease;
}
.hist-enter-from {
  opacity: 0;
  transform: translateX(-12px);
}
.hist-leave-to {
  opacity: 0;
  transform: translateX(8px);
}
</style>
