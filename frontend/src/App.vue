<script setup>
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import {
  computed,
  nextTick,
  onMounted,
  onUnmounted,
  ref,
  watch,
} from "vue";
import { loadSpoorboekje, saveSpoorboekje } from "./spoorboekjeidb.js";

const ROUTE_TTL_MS = 5000;
const MAX_HISTORY = 80;
const SPOORBOEKJE_PAGE_SIZE = 10;

const mapEl = ref(null);
let map;
let ws;
let contactLayerGroup;
let selfMarker;

/** Live routes (verdwijnen na TTL) */
const routes = ref([]);
/** Gearchiveerde paden na TTL — blijven in lijst */
const routeHistory = ref([]);
/** Aantal stations in server-index (kaart-API); geen volledige lijst meer via WebSocket. */
const contactReportedTotal = ref(null);
const selfPos = ref(null);
const status = ref("Verbinden…");
/** Pauze: geen nieuwe routes/contact-updates van WebSocket verwerken */
const livePaused = ref(false);
/** Contactenlijst in-/uitklappen */
const contactsExpanded = ref(false);
/** Spoorboekje (archief) in-/uitklappen */
const archiveExpanded = ref(false);
/** id's van archief-paden die nu op de kaart getekend zijn */
const selectedHistoryIds = ref([]);
/** 1-based pagina in het spoorboekje */
const spoorboekjePage = ref(1);

/** Live: id -> polyline */
const routeLayers = new Map();
/** Live: id -> timeout */
const routeTimeouts = new Map();
/** Archief: id -> polyline (alleen als geselecteerd) */
const historyRouteLayers = new Map();

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

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
  const s = 65 + Math.random() * 30;
  const l = 42 + Math.random() * 16;
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

/** Volledige hop-hex uit het pakket (1–4 bytes) voor labels; match met `pubkey_prefix_hex`. */
function hopHexLabel(h) {
  return String(h ?? "")
    .trim()
    .toLowerCase()
    .replace(/^0x/, "");
}

/** Stationsnaam: match langste gemeenschappelijke hex-prefix met `pubkey_prefix_hex` (zelfde logica als backend). */
function stationLabelForHop(hopHex, list) {
  const raw = String(hopHex ?? "")
    .trim()
    .toLowerCase();
  if (!raw || !/^[0-9a-f]+$/i.test(raw)) {
    return "?";
  }
  const h = raw.length % 2 === 1 ? `0${raw}` : raw;
  const matches = list.filter((c) => {
    const pk = String(c.pubkey_prefix_hex || "").toLowerCase();
    return pk.startsWith(h);
  });
  if (matches.length === 0 && h.length > 2) {
    const short = h.slice(0, 2);
    const fb = list.filter((c) =>
      String(c.pubkey_prefix_hex || "").toLowerCase().startsWith(short),
    );
    if (fb.length) {
      const withGps = fb.find(
        (c) =>
          c.lat != null &&
          c.lon != null &&
          Number.isFinite(Number(c.lat)) &&
          Number.isFinite(Number(c.lon)),
      );
      const c = withGps || fb[0];
      return (
        (c.name && String(c.name).trim()) || c.pubkey_prefix_hex || short
      );
    }
  }
  if (matches.length === 0) {
    return h;
  }
  const withGps = matches.find(
    (c) =>
      c.lat != null &&
      c.lon != null &&
      Number.isFinite(Number(c.lat)) &&
      Number.isFinite(Number(c.lon)),
  );
  const c = withGps || matches[0];
  const name =
    (c.name && String(c.name).trim()) || c.pubkey_prefix_hex || h;
  return name;
}

/** Lijst van knooppunten (volgnummer, hop-hex, naam) — uit `hop_steps` van de backend. */
function routeHopNodes(route) {
  const steps = Array.isArray(route?.hop_steps) ? route.hop_steps : [];
  if (steps.length) {
    return steps.map((step, i) => ({
      n: i + 1,
      hopHex: hopHexLabel(step.hop_hex ?? route.hops_hex?.[i]),
      name:
        (step.name && String(step.name).trim()) ||
        step.pubkey_prefix_hex ||
        hopHexLabel(step.hop_hex) ||
        "?",
    }));
  }
  const hops = Array.isArray(route?.hops_hex) ? route.hops_hex : [];
  return hops.map((hop, i) => ({
    n: i + 1,
    hopHex: hopHexLabel(hop),
    name: stationLabelForHop(hop, []),
  }));
}

/** Eigen marker; hop-knooppunten op live + geselecteerde archief-paden in `renderRouteStationMarkers`. */
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

/**
 * Blauwe knooppunt-markers op **live** paden en op **getoonde** spoorboekje-paden.
 */
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

/**
 * TTL verlopen: live poly weg, snapshot naar archief.
 */
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

function isHistorySelected(id) {
  return selectedHistoryIds.value.includes(String(id));
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
    color: entry.routeColor || "#94a3b8",
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

const nowTick = ref(0);
let relTimeTimer;

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

/** Relatieve tijd herberekent als `nowTick` verandert. */
function timeLabel(entry) {
  void nowTick.value;
  return formatRelative(entry.archivedAt);
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

  relTimeTimer = setInterval(() => {
    nowTick.value += 1;
  }, 10000);

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
  clearInterval(relTimeTimer);
  for (const id of [...routeLayers.keys()]) {
    removeLiveLayerOnly(id);
  }
  routes.value = [];
  clearAllHistoryOverlays();
  /** Spoorboekje blijft in IndexedDB; geen reset hier. */
  ws?.close();
  map?.remove();
});
</script>

<template>
  <div class="layout">
    <aside class="panel">
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
          @click="livePaused = !livePaused"
        >
          <span class="btn-pause-icon" aria-hidden="true">{{
            livePaused ? "▶" : "⏸"
          }}</span>
          {{ livePaused ? "Hervat live updates" : "Pauzeer live updates" }}
        </button>
        <p v-if="livePaused" class="pause-hint">
          Nieuwe paden en index-/positie-updates worden niet verwerkt (bestaande
          timers lopen door).
        </p>
      </div>
      <p class="hint">
        Live paden vervagen na {{ ROUTE_TTL_MS / 1000 }}s — ze belanden in het
        <strong>spoorboekje</strong>. Tik een kaart om het pad weer op de kaart
        te leggen; tik nogmaals om het te verbergen.
      </p>
      <p v-if="selfPos?.length === 2" class="self-pos">
        Zelf (advert): {{ selfPos[0].toFixed(4) }}°, {{ selfPos[1].toFixed(4) }}°
      </p>

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
            <span class="id">{{ r.id }}</span>
            <span class="pill pill-live">live</span>
          </div>
          <span class="hops">{{ (r.hops_hex || []).join(" → ") }}</span>
          <!-- <ol v-if="routeHopNodes(r).length" class="route-nodes">
            <li v-for="node in routeHopNodes(r)" :key="`${r.id}-hop-${node.n}`">
              <span class="route-node-badge" aria-hidden="true">{{ node.n }}</span>
              <span class="route-node-name">{{ node.name }}</span>
              <span v-if="node.hopHex" class="route-node-hex"
                >{{ node.hopHex }}</span
              >
            </li>
          </ol> -->
          <span v-if="r.snr != null" class="meta"
            >SNR {{ Number(r.snr).toFixed(1) }}</span
          >
        </li>
        <li v-if="routes.length === 0" class="empty"
          >Wacht op RF-log — nog geen live spoor</li
        >
      </ul>

      <div class="archive-block">
        <div class="archive-toolbar">
          <button
            type="button"
            class="archive-toggle"
            :aria-expanded="archiveExpanded"
            aria-controls="archive-panel"
            id="archive-heading"
            @click="archiveExpanded = !archiveExpanded"
          >
            <span class="h2-icon archive" aria-hidden="true" />
            <span class="archive-toggle-title">Spoorboekje</span>
            <span class="archive-count">{{ routeHistory.length }}</span>
            <span
              v-if="selectedHistoryIds.length > 0"
              class="archive-selected-badge"
              :title="selectedHistoryIds.length + ' pad(en) op de kaart'"
              >{{ selectedHistoryIds.length }}×</span
            >
            <span class="archive-chevron" aria-hidden="true">{{
              archiveExpanded ? "▾" : "▸"
            }}</span>
          </button>
          <button
            v-if="selectedHistoryIds.length > 0"
            type="button"
            class="btn-ghost btn-ghost--sm"
            @click.stop="clearAllHistoryOverlays"
          >
            Verberg
          </button>
        </div>
        <div
          v-show="archiveExpanded"
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
            v-if="routeHistory.length > 0 && spoorboekjeTotalPages > 0"
            class="archive-pagination"
            role="navigation"
            aria-label="Spoorboekje paginering"
          >
            <button
              type="button"
              class="btn-ghost btn-ghost--sm btn-page"
              :disabled="spoorboekjePage <= 1"
              @click="spoorboekjePrev"
            >
              Vorige
            </button>
            <span class="archive-pagination-label">
              Pagina {{ spoorboekjePage }} van {{ spoorboekjeTotalPages }}
              <span class="archive-pagination-range">
                ({{ (spoorboekjePage - 1) * SPOORBOEKJE_PAGE_SIZE + 1 }}–{{
                  Math.min(
                    spoorboekjePage * SPOORBOEKJE_PAGE_SIZE,
                    routeHistory.length,
                  )
                }}
                van {{ routeHistory.length }})
              </span>
            </span>
            <button
              type="button"
              class="btn-ghost btn-ghost--sm btn-page"
              :disabled="spoorboekjePage >= spoorboekjeTotalPages"
              @click="spoorboekjeNext"
            >
              Volgende
            </button>
          </div>
          <div v-if="routeHistory.length > 0" class="archive-actions">
            <button
              type="button"
              class="btn-clear-archive"
              @click="clearSpoorboekje"
            >
              Spoorboekje legen
            </button>
          </div>

          <TransitionGroup name="hist" tag="ul" class="history-rail">
            <li
              v-for="entry in historyPageSlice"
              :key="entry.id + '-' + entry.archivedAt"
              class="history-card"
              :class="{ 'history-card--on': isHistorySelected(entry.id) }"
            >
              <button
                type="button"
                class="history-card-btn"
                :aria-pressed="isHistorySelected(entry.id)"
                @click="toggleHistoryRoute(entry)"
              >
                <span class="history-rail-line" aria-hidden="true" />
                <span
                  class="history-swatch"
                  :style="{ background: entry.routeColor || '#64748b' }"
                />
                <div class="history-body">
                  <div class="history-top">
                    <span class="id">{{ entry.id }}</span>
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
                  isHistorySelected(entry.id) ? "✓" : "+"
                }}</span>
              </button>
            </li>
          </TransitionGroup>
          <p v-if="routeHistory.length === 0" class="empty archive-empty">
            Nog geen archief — live paden komen hier automatisch terecht.
          </p>

          <button
            v-if="routeHistory.length > 0 && selectedHistoryIds.length > 0"
            type="button"
            class="btn-fit"
            @click="fitAllVisibleRoutes"
          >
            Zoom: alle zichtbare paden
          </button>
        </div>
      </div>

      <div class="contacts-block">
        <button
          type="button"
          class="contacts-toggle"
          :aria-expanded="contactsExpanded"
          aria-controls="contacts-panel"
          id="contacts-heading"
          @click="contactsExpanded = !contactsExpanded"
        >
          <span class="h2-icon contact" aria-hidden="true" />
          <span class="contacts-toggle-title">Contacten</span>
          <span
            class="contacts-count"
            :title="
              contactReportedTotal != null
                ? 'Kaart-index map.meshcore.io — unieke pubkey-prefixen'
                : ''
            "
            >{{ contactReportedTotal ?? "—" }}</span
          >
          <span class="contacts-chevron" aria-hidden="true">{{
            contactsExpanded ? "▾" : "▸"
          }}</span>
        </button>
        <div
          v-show="contactsExpanded"
          id="contacts-panel"
          class="contacts-panel"
          role="region"
          aria-labelledby="contacts-heading"
        >
          <p class="contacts-summary">
            <strong>{{ contactReportedTotal ?? "—" }}</strong>
            <span class="contacts-summary-label"> stations in kaart-index (server)</span>
          </p>
        </div>
      </div>
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
.h2-icon.archive {
  background: linear-gradient(135deg, #a78bfa, #6366f1);
  box-shadow: 0 0 8px rgba(167, 139, 250, 0.35);
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
.contacts-block {
  margin-top: 1rem;
}
.contacts-toggle {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 0.4rem;
  font: inherit;
  padding: 0.45rem 0.4rem;
  border-radius: 8px;
  border: 1px solid #2e2a40;
  background: rgba(59, 130, 246, 0.08);
  color: #e8e8ef;
  cursor: pointer;
  text-align: left;
}
.contacts-toggle:hover {
  background: rgba(59, 130, 246, 0.14);
  border-color: #3f4a6b;
}
.contacts-toggle-title {
  flex: 1;
  font-size: 0.72rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.09em;
  color: #8b9cb8;
}
.contacts-count {
  font-size: 0.68rem;
  font-weight: 600;
  color: #9fb0c8;
  background: rgba(255, 255, 255, 0.06);
  padding: 0.1rem 0.4rem;
  border-radius: 999px;
}
.contacts-chevron {
  color: #7a8699;
  font-size: 0.75rem;
}
.contacts-panel {
  margin-top: 0.35rem;
  padding-left: 0.45rem;
  border-left: 2px solid rgba(59, 130, 246, 0.28);
}
.contacts-summary {
  margin: 0;
  font-size: 0.82rem;
  color: #c9cdd6;
  line-height: 1.45;
}
.contacts-summary-label {
  color: #8b95a8;
  font-weight: 400;
}
.h2-icon.contact {
  background: linear-gradient(135deg, #60a5fa, #2563eb);
  box-shadow: 0 0 8px rgba(96, 165, 250, 0.4);
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

.meta.no-gps {
  color: #a78bfa;
}
.map {
  flex: 1;
  min-height: 200px;
}

/* Permanente stationslabels bij contact-markers (Leaflet tooltip) */
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
