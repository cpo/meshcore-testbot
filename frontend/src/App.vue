<script setup>
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import ContactsPanel from "./components/ContactsPanel.vue";
import LiveRoutesList from "./components/LiveRoutesList.vue";
import SpoorboekjePanel from "./components/SpoorboekjePanel.vue";
import VisorHeader from "./components/VisorHeader.vue";
import {
  MAX_HISTORY,
  ROUTE_TTL_MS,
  SPOORBOEKJE_PAGE_SIZE,
} from "./constants.js";
import { loadSpoorboekje, saveSpoorboekje } from "./spoorboekjeidb.js";
import {
  escapeHtml,
  hopHexLabel,
  stationLabelForHop,
} from "./utils/routeFormat.js";

const routeTtlSec = ROUTE_TTL_MS / 1000;

const mapEl = ref(null);
let map;
let ws;
let contactLayerGroup;
let selfMarker;

const routes = ref([]);
const routeHistory = ref([]);
const contactReportedTotal = ref(null);
const selfPos = ref(null);
const status = ref("Verbinden…");
const livePaused = ref(false);
const contactsExpanded = ref(false);
const archiveExpanded = ref(false);
const selectedHistoryIds = ref([]);
const spoorboekjePage = ref(1);

const routeLayers = new Map();
const routeTimeouts = new Map();
const historyRouteLayers = new Map();

function wsUrl() {
  const explicit = import.meta.env.VITE_VISOR_WS_URL;
  if (explicit) {
    return String(explicit);
  }
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}/ws`;
}

function randomRouteColor() {
  const h = Math.random() * 360;
  // Higher saturation, lower lightness so polylines stay readable on light basemaps.
  const s = 72 + Math.random() * 24;
  const l = 26 + Math.random() * 14;
  return `hsl(${h} ${s}% ${l}%)`;
}

function fitMapToPolyline(latlngs) {
  if (!latlngs.length || !map) {
    return;
  }
  try {
    if (latlngs.length === 1) {
      map.setView(latlngs[0], 14);
      return;
    }
    const bounds = L.latLngBounds(latlngs);
    if (bounds.isValid()) {
      map.fitBounds(bounds, { padding: [48, 48], maxZoom: 14 });
    }
  } catch {
    /* ignore */
  }
}

function fitAllVisibleRoutes() {
  if (!map) {
    return;
  }
  let combined = null;
  const merge = (b) => {
    if (!b?.isValid?.()) {
      return;
    }
    if (combined === null) {
      combined = L.latLngBounds(b.getSouthWest(), b.getNorthEast());
    } else {
      combined.extend(b);
    }
  };
  for (const layer of routeLayers.values()) {
    merge(layer.getBounds?.());
  }
  for (const layer of historyRouteLayers.values()) {
    merge(layer.getBounds?.());
  }
  if (contactLayerGroup) {
    contactLayerGroup.eachLayer((layer) => {
      const ll = layer.getLatLng?.();
      if (ll) {
        merge(L.latLngBounds(ll, ll));
      }
    });
  }
  if (combined?.isValid?.()) {
    map.fitBounds(combined, { padding: [56, 56], maxZoom: 14 });
  }
}

function updateSelfMarker(selfLatLon) {
  if (!map) {
    return;
  }
  if (selfMarker) {
    map.removeLayer(selfMarker);
    selfMarker = null;
  }
  if (
    selfLatLon &&
    Array.isArray(selfLatLon) &&
    selfLatLon.length === 2 &&
    Number.isFinite(selfLatLon[0]) &&
    Number.isFinite(selfLatLon[1])
  ) {
    const [lat, lon] = selfLatLon;
    selfMarker = L.circleMarker([lat, lon], {
      radius: 10,
      color: "#22c55e",
      weight: 2,
      fillColor: "#22c55e",
      fillOpacity: 0.85,
    })
      .bindPopup("Zelf (advert-locatie)")
      .addTo(map);
  }
}

function renderRouteStationMarkers() {
  if (!map) {
    return;
  }
  if (!contactLayerGroup) {
    contactLayerGroup = L.layerGroup().addTo(map);
  }
  contactLayerGroup.clearLayers();

  function drawNodesForRoute(r) {
    const hops = Array.isArray(r.hops_hex) ? r.hops_hex : [];
    const coords = Array.isArray(r.coords) ? r.coords : [];
    const steps = Array.isArray(r.hop_steps) ? r.hop_steps : [];
    const n = Math.min(hops.length, coords.length);
    for (let i = 0; i < n; i++) {
      const hop = hops[i];
      const step = steps[i];
      const pt = coords[i];
      if (!Array.isArray(pt) || pt.length < 2) {
        continue;
      }
      const lat = Number(pt[0]);
      const lon = Number(pt[1]);
      if (!Number.isFinite(lat) || !Number.isFinite(lon)) {
        continue;
      }
      let station;
      if (step) {
        const nm = step.name != null && String(step.name).trim();
        station = nm
          ? nm
          : step.pubkey_prefix_hex ||
            hopHexLabel(step.hop_hex || hop) ||
            hopHexLabel(hop);
      } else {
        station = stationLabelForHop(hop, []);
      }
      const pkHint = hopHexLabel(step?.hop_hex ?? hop);
      const idxLabel = `${i + 1}/${n}`;
      const tip = `${idxLabel} ${station}`;
      L.circleMarker([lat, lon], {
        radius: 6,
        color: "#3b82f6",
        weight: 2,
        fillColor: "#60a5fa",
        fillOpacity: 0.9,
      })
        .bindTooltip(escapeHtml(tip), {
          permanent: true,
          direction: "right",
          offset: [6, 0],
          className: "contact-station-label",
        })
        .bindPopup(
          `<b>Knooppunt ${i + 1} van ${n}</b><br>${escapeHtml(station)}<br><span class="popup-hop-hint">hop ${escapeHtml(pkHint)}</span>`,
        )
        .addTo(contactLayerGroup);
    }
  }

  for (const r of routes.value) {
    drawNodesForRoute(r);
  }
  for (const hid of selectedHistoryIds.value) {
    const entry = routeHistory.value.find((e) => String(e.id) === String(hid));
    if (entry) {
      drawNodesForRoute(entry);
    }
  }
}

function renderSelfAndRoutes(selfLatLon) {
  updateSelfMarker(selfLatLon);
  renderRouteStationMarkers();
}

function removeLiveLayerOnly(id) {
  const tid = routeTimeouts.get(id);
  if (tid !== undefined) {
    clearTimeout(tid);
    routeTimeouts.delete(id);
  }
  const layer = routeLayers.get(id);
  if (layer && map) {
    map.removeLayer(layer);
  }
  routeLayers.delete(id);
}

function archiveLiveRoute(id) {
  const sid = String(id);
  removeLiveLayerOnly(sid);
  const row = routes.value.find((r) => String(r.id) === sid);
  routes.value = routes.value.filter((r) => String(r.id) !== sid);
  if (!row || !Array.isArray(row.coords) || !row.coords.length) {
    renderRouteStationMarkers();
    return;
  }
  const entry = {
    ...row,
    archivedAt: Date.now(),
  };
  const rest = routeHistory.value.filter((r) => String(r.id) !== sid);
  routeHistory.value = [entry, ...rest].slice(0, MAX_HISTORY);
  renderRouteStationMarkers();
}

function handleRouteMessage(data) {
  if (!Array.isArray(data.coords) || !data.coords.length || !map) {
    return;
  }
  const latlngs = data.coords.map(([lat, lon]) => [lat, lon]);
  const id = String(data.id);
  removeLiveLayerOnly(id);
  const color = randomRouteColor();
  const poly = L.polyline(latlngs, {
    color,
    weight: 4,
    opacity: 0.95,
  }).addTo(map);
  routeLayers.set(id, poly);

  routes.value = [
    ...routes.value.filter((r) => String(r.id) !== id),
    { ...data, routeColor: color },
  ];

  fitMapToPolyline(latlngs);
  map.invalidateSize();

  const tid = setTimeout(() => archiveLiveRoute(id), ROUTE_TTL_MS);
  routeTimeouts.set(id, tid);

  renderRouteStationMarkers();
}

function toggleHistoryRoute(entry) {
  const id = String(entry.id);
  if (!map || !Array.isArray(entry.coords) || !entry.coords.length) {
    return;
  }
  if (historyRouteLayers.has(id)) {
    const layer = historyRouteLayers.get(id);
    if (layer) {
      map.removeLayer(layer);
    }
    historyRouteLayers.delete(id);
    selectedHistoryIds.value = selectedHistoryIds.value.filter((x) => x !== id);
    renderRouteStationMarkers();
    return;
  }
  const latlngs = entry.coords.map(([lat, lon]) => [lat, lon]);
  const poly = L.polyline(latlngs, {
    color: entry.routeColor || "#475569",
    weight: 5,
    opacity: 0.88,
    dashArray: "10 8",
    lineCap: "round",
    lineJoin: "round",
  }).addTo(map);
  historyRouteLayers.set(id, poly);
  selectedHistoryIds.value = [...selectedHistoryIds.value, id];
  fitMapToPolyline(latlngs);
  renderRouteStationMarkers();
}

function clearAllHistoryOverlays() {
  for (const id of [...historyRouteLayers.keys()]) {
    const layer = historyRouteLayers.get(id);
    if (layer && map) {
      map.removeLayer(layer);
    }
    historyRouteLayers.delete(id);
  }
  selectedHistoryIds.value = [];
  renderRouteStationMarkers();
}

function clearSpoorboekje() {
  if (routeHistory.value.length === 0) {
    return;
  }
  if (
    !confirm(
      "Alle gearchiveerde paden uit het spoorboekje verwijderen? Dit wist ook paden die nu op de kaart staan.",
    )
  ) {
    return;
  }
  clearAllHistoryOverlays();
  routeHistory.value = [];
  void saveSpoorboekje([]);
}

const historySorted = computed(() => [...routeHistory.value]);

const spoorboekjeTotalPages = computed(() => {
  const n = routeHistory.value.length;
  if (n === 0) {
    return 0;
  }
  return Math.ceil(n / SPOORBOEKJE_PAGE_SIZE);
});

const historyPageSlice = computed(() => {
  const all = historySorted.value;
  const tp = spoorboekjeTotalPages.value;
  if (tp === 0) {
    return [];
  }
  const page = Math.min(Math.max(1, spoorboekjePage.value), tp);
  const start = (page - 1) * SPOORBOEKJE_PAGE_SIZE;
  return all.slice(start, start + SPOORBOEKJE_PAGE_SIZE);
});

function spoorboekjePrev() {
  if (spoorboekjePage.value > 1) {
    spoorboekjePage.value -= 1;
  }
}

function spoorboekjeNext() {
  const tp = spoorboekjeTotalPages.value;
  if (tp > 0 && spoorboekjePage.value < tp) {
    spoorboekjePage.value += 1;
  }
}

watch(
  routeHistory,
  (entries) => {
    void saveSpoorboekje(entries);
  },
  { deep: true },
);

watch(
  () => routeHistory.value.length,
  () => {
    const tp = spoorboekjeTotalPages.value;
    if (tp === 0) {
      spoorboekjePage.value = 1;
      return;
    }
    if (spoorboekjePage.value > tp) {
      spoorboekjePage.value = tp;
    }
  },
);

onMounted(async () => {
  map = L.map(mapEl.value).setView([51.7, 5.3], 8);
  L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", {
    attribution: "© OpenStreetMap-bijdragers",
    maxZoom: 19,
  }).addTo(map);

  await nextTick();
  requestAnimationFrame(() => {
    map.invalidateSize();
  });

  try {
    const loaded = await loadSpoorboekje(MAX_HISTORY);
    routeHistory.value = loaded;
  } catch {
    /* ignore */
  }

  const url = wsUrl();
  ws = new WebSocket(url);
  ws.onopen = () => {
    status.value = `Verbonden (${url})`;
  };
  ws.onclose = () => {
    status.value = "WebSocket gesloten";
  };
  ws.onerror = () => {
    status.value = `WebSocket-fout (${url}) — draait de bot op poort 3847?`;
  };
  ws.onmessage = (ev) => {
    try {
      const data = JSON.parse(ev.data);
      if (livePaused.value) {
        if (data.type === "contacts" || data.type === "route") {
          return;
        }
      }
      if (data.type === "contacts") {
        selfPos.value = data.self_pos ?? null;
        contactReportedTotal.value =
          data.reported_total != null && Number.isFinite(Number(data.reported_total))
            ? Number(data.reported_total)
            : null;
        renderSelfAndRoutes(selfPos.value);
        return;
      }
      if (data.type === "route") {
        handleRouteMessage(data);
      }
    } catch {
      /* ignore */
    }
  };
});

onUnmounted(() => {
  void saveSpoorboekje(routeHistory.value);
  for (const id of [...routeLayers.keys()]) {
    removeLiveLayerOnly(id);
  }
  routes.value = [];
  clearAllHistoryOverlays();
  ws?.close();
  map?.remove();
});
</script>

<template>
  <div class="layout">
    <aside class="panel">
      <VisorHeader
        :status="status"
        :live-paused="livePaused"
        :self-pos="selfPos"
        :route-ttl-sec="routeTtlSec"
        @toggle-pause="livePaused = !livePaused"
      />

      <LiveRoutesList :routes="routes" />

      <SpoorboekjePanel
        v-model:expanded="archiveExpanded"
        :route-history-count="routeHistory.length"
        :selected-history-ids="selectedHistoryIds"
        :spoorboekje-page="spoorboekjePage"
        :spoorboekje-total-pages="spoorboekjeTotalPages"
        :history-page-slice="historyPageSlice"
        :page-size="SPOORBOEKJE_PAGE_SIZE"
        @clear-overlays="clearAllHistoryOverlays"
        @clear-spoorboekje="clearSpoorboekje"
        @fit-all="fitAllVisibleRoutes"
        @prev-page="spoorboekjePrev"
        @next-page="spoorboekjeNext"
        @toggle-history="toggleHistoryRoute"
      />

      <ContactsPanel
        v-model:expanded="contactsExpanded"
        :reported-total="contactReportedTotal"
      />
    </aside>
    <div ref="mapEl" class="map" />
  </div>
</template>

<style>
html,
body {
  margin: 0;
  height: 100%;
}
.layout {
  display: flex;
  height: 100vh;
  font-family:
    "DM Sans",
    system-ui,
    -apple-system,
    sans-serif;
}
.panel {
  width: min(360px, 44vw);
  padding: 1rem 1rem 1.25rem;
  background: linear-gradient(165deg, #0f1118 0%, #12121c 40%, #0c0e14 100%);
  color: #e8e8ef;
  overflow: auto;
  border-right: 1px solid #2a2a3a;
  box-shadow: 4px 0 24px rgba(0, 0, 0, 0.35);
}
.map {
  flex: 1;
  min-height: 200px;
}

.leaflet-tooltip.contact-station-label {
  background: rgba(15, 17, 24, 0.72);
  color: rgba(232, 232, 239, 0.68);
  border: 1px solid rgba(96, 165, 250, 0.28);
  border-radius: 6px;
  padding: 1px 6px;
  font-size: 0.58rem;
  font-weight: 500;
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.22);
  max-width: 200px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.leaflet-popup-content .popup-hop-hint {
  display: block;
  margin-top: 0.35rem;
  font-size: 0.72rem;
  color: #7a8699;
}
</style>
