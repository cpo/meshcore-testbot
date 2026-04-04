/**
 * Spoorboekje (archiefroutes) in IndexedDB — blijft bestaan na herlaad.
 *
 * In DevTools: database `meshcore-visor` → store `spoorboekje` → één record
 * met key `entries` (waarde = JSON-array van routes), niet één rij per route.
 */

const DB_NAME = "meshcore-visor";
const DB_VERSION = 1;
const STORE = "spoorboekje";
const KEY = "entries";

function idbSupported() {
  return typeof indexedDB !== "undefined";
}

function openDb() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onerror = () => reject(req.error ?? new Error("indexedDB.open failed"));
    req.onsuccess = () => resolve(req.result);
    req.onupgradeneeded = (e) => {
      const db = e.target.result;
      if (!db.objectStoreNames.contains(STORE)) {
        db.createObjectStore(STORE);
      }
    };
  });
}

/** Vue reactive arrays/objects zijn geen geldige structured-clone input voor IDB. */
function clonePlainForIdb(entries) {
  try {
    return JSON.parse(JSON.stringify(entries));
  } catch {
    return [];
  }
}

function isValidEntry(x) {
  return (
    x != null &&
    typeof x === "object" &&
    x.id != null &&
    Array.isArray(x.coords) &&
    x.coords.length > 0 &&
    typeof x.archivedAt === "number" &&
    Number.isFinite(x.archivedAt)
  );
}

/**
 * @param {number} maxEntries
 * @returns {Promise<object[]>}
 */
export async function loadSpoorboekje(maxEntries) {
  if (!idbSupported()) {
    return [];
  }
  let db;
  try {
    db = await openDb();
  } catch {
    return [];
  }
  return new Promise((resolve) => {
    const tx = db.transaction(STORE, "readonly");
    tx.oncomplete = () => {
      db.close();
    };
    tx.onerror = () => {
      db.close();
      resolve([]);
    };
    const store = tx.objectStore(STORE);
    const req = store.get(KEY);
    req.onsuccess = () => {
      const v = req.result;
      if (!Array.isArray(v)) {
        resolve([]);
        return;
      }
      const cleaned = v.filter(isValidEntry).slice(0, maxEntries);
      resolve(cleaned);
    };
    req.onerror = () => {
      resolve([]);
    };
  });
}

/**
 * @param {object[]} entries
 * @returns {Promise<void>}
 */
export async function saveSpoorboekje(entries) {
  if (!idbSupported()) {
    return;
  }
  const plain = Array.isArray(entries) ? clonePlainForIdb(entries) : [];
  let db;
  try {
    db = await openDb();
  } catch {
    return;
  }
  return new Promise((resolve) => {
    const tx = db.transaction(STORE, "readwrite");
    tx.oncomplete = () => {
      db.close();
      resolve();
    };
    tx.onerror = () => {
      db.close();
      resolve();
    };
    tx.onabort = () => {
      db.close();
      resolve();
    };
    const store = tx.objectStore(STORE);
    store.put(plain, KEY);
  });
}
