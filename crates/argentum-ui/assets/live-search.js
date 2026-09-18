// Keystroke-live search debounce for Argentum tables (GH #172).
//
// The live search input (`data-live-search-input`) is deliberately unbound:
// typing stays local until it pauses, so a burst like "published" triggers
// one grid reload instead of nine. After `data-debounce-ms` milliseconds of
// quiet the script copies the value into the bound hidden transport
// (`data-live-search-transport`) and dispatches a bubbling `change` into it,
// which the runtime turns into signal writes — the shard re-renders the grid
// in place exactly as if the user had typed into a bound input, so the
// abort-in-flight coalescing still applies to the resulting rerun. Pressing
// Enter flushes the pending value immediately instead of waiting out the
// timer. Without JS the `<noscript>` GET form is the search path and this
// script never runs.
//
// Document-level delegation (like filters.js) so streamed/shard swaps that
// replace table markup need no re-installation. Per-input timers live in a
// WeakMap keyed by the visible input; a timer firing for a detached (swapped
// out) input is dropped, so a stale value can never overwrite a newer one.
const timers = new WeakMap();

function transportFor(input) {
  const host = input.closest('[data-live-search]');
  if (!host) return null;
  return host.querySelector('[data-live-search-transport]');
}

function flush(input) {
  const transport = transportFor(input);
  if (!transport || !input.isConnected || !transport.isConnected) return;
  if (transport.value !== input.value) {
    transport.value = input.value;
    transport.dispatchEvent(new Event('change', { bubbles: true }));
  }
}

function debounceMs(input) {
  const raw = parseInt(input.getAttribute('data-debounce-ms') || '200', 10);
  return Number.isFinite(raw) && raw >= 0 ? raw : 200;
}

document.addEventListener('input', (e) => {
  const input = e.target.closest('[data-live-search-input]');
  if (!input) return;
  if (!transportFor(input)) return;
  const pending = timers.get(input);
  if (pending) clearTimeout(pending);
  timers.set(
    input,
    setTimeout(() => {
      timers.delete(input);
      flush(input);
    }, debounceMs(input)),
  );
});

// Enter means "search now": flush instead of waiting out the timer.
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Enter') return;
  const input = e.target.closest('[data-live-search-input]');
  if (!input) return;
  const pending = timers.get(input);
  if (pending) {
    clearTimeout(pending);
    timers.delete(input);
  }
  flush(input);
});
