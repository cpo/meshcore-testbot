<script setup>
const expanded = defineModel("expanded", { type: Boolean, default: false });

defineProps({
  /** @type {number | null} */
  reportedTotal: { default: null },
});
</script>

<template>
  <div class="contacts-block">
    <button
      type="button"
      class="contacts-toggle"
      :aria-expanded="expanded"
      aria-controls="contacts-panel"
      id="contacts-heading"
      @click="expanded = !expanded"
    >
      <span class="h2-icon contact" aria-hidden="true" />
      <span class="contacts-toggle-title">Contacten</span>
      <span
        class="contacts-count"
        :title="
          reportedTotal != null
            ? 'Kaart-index map.meshcore.io — unieke pubkey-prefixen'
            : ''
        "
        >{{ reportedTotal ?? "—" }}</span
      >
      <span class="contacts-chevron" aria-hidden="true">{{
        expanded ? "▾" : "▸"
      }}</span>
    </button>
    <div
      v-show="expanded"
      id="contacts-panel"
      class="contacts-panel"
      role="region"
      aria-labelledby="contacts-heading"
    >
      <p class="contacts-summary">
        <strong>{{ reportedTotal ?? "—" }}</strong>
        <span class="contacts-summary-label"> stations in kaart-index (server)</span>
      </p>
    </div>
  </div>
</template>

<style scoped>
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
.h2-icon {
  width: 8px;
  height: 8px;
  border-radius: 2px;
  flex-shrink: 0;
}
.h2-icon.contact {
  background: linear-gradient(135deg, #60a5fa, #2563eb);
  box-shadow: 0 0 8px rgba(96, 165, 250, 0.4);
}
</style>
