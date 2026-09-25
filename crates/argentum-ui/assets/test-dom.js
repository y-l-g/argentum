// A minimal `document` stand-in for the asset tests.
//
// The shipped scripts are plain browser scripts: `install()` reads `document`
// (and sometimes `window`) from the global scope and every handler is
// document-delegated, so a test has to put a stand-in in place before requiring
// the script. Every stand-in needs the same listener registry; whatever else a
// script reads (`querySelectorAll`, `getElementById`, `createElement`, …) is
// the caller's override.

// Build a stand-in `document` that records the listeners `install()` registers.
//
// `listeners(type)` returns them in registration order — firing them all is
// what a browser does for one event — and `types()` names every type a handler
// was registered for.
function listenerDocument(overrides = {}) {
  const byType = new Map();
  return {
    addEventListener(type, handler) {
      if (!byType.has(type)) byType.set(type, []);
      byType.get(type).push(handler);
    },
    listeners(type) {
      return byType.get(type) || [];
    },
    types() {
      return Array.from(byType.keys());
    },
    ...overrides,
  };
}

module.exports = { listenerDocument };
