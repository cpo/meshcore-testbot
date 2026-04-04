export function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** Route `id` from server is usually ms since epoch; show as UTC ISO. */
export function routeIdUtc(id) {
  const raw = id == null ? "" : String(id).trim();
  const n = Number(raw);
  if (!Number.isFinite(n)) {
    return raw;
  }
  const d = new Date(n);
  if (Number.isNaN(d.getTime())) {
    return raw;
  }
  if (n < 1e11 || n > 1e14) {
    return raw;
  }
  return d.toISOString();
}

export function hopHexLabel(h) {
  return String(h ?? "")
    .trim()
    .toLowerCase()
    .replace(/^0x/, "");
}

export function stationLabelForHop(hopHex, list) {
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

export function routeHopNodes(route) {
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
