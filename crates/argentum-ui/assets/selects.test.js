// Unit tests for `selects.js` (GH #184, GH #236, GH #237).
//
// There is no JS test runner in this workspace — the assets are plain browser
// scripts loaded through `asset!` — so this runs on Node's built-in runner and
// reaches the script through the guarded `module.exports` at the bottom of the
// script:
//
//     cargo test -p argentum-ui            # renders and Rust-side assertions
//     node --test crates/argentum-ui/assets/selects.test.js
//
// What the cases protect:
// * the matching rule GH #184 filed the list for, including the two easy
//   mistakes — case sensitivity and letting the placeholder eat the cap;
// * the hide decision behind GH #236: the native `<select>` leaves the display
//   only when the combobox that replaces it is wired, and stays the submitted
//   value carrier either way;
// * the single wiring pass behind GH #237. The document stand-in below records
//   the listeners `install()` registers and fires them the way a browser does
//   — every listener for the type, in registration order — so a duplicated
//   wiring block shows up as two `change` events per activation and two rows
//   moved per arrow key.

const test = require('node:test');
const assert = require('node:assert/strict');

const SCRIPT = require.resolve('./selects.js');

// --- a document stand-in -----------------------------------------------------

// `selects.js` is a plain browser script: `install()` reads `document` from the
// global scope and every handler is document-delegated, so the stand-in has to
// be in place before the script is required. It is only as wide as the script
// needs: each node answers the selectors `partsOf` and the listbox read.
function standInDocument(filters) {
  const byType = new Map();
  return {
    activeElement: null,
    documentElement: null,
    addEventListener(type, handler) {
      if (!byType.has(type)) byType.set(type, []);
      byType.get(type).push(handler);
    },
    querySelectorAll(selector) {
      return selector === '[data-options-filter]' ? filters : [];
    },
    // Every listener for `type`, in registration order: firing them all is what
    // a browser does for one event.
    listeners(type) {
      return byType.get(type) || [];
    },
    types() {
      return Array.from(byType.keys());
    },
  };
}

// Load a fresh copy of the script against `document`. A fresh copy re-runs
// `install()`, so each case gets its own listener set and its own initial hide
// pass.
function load(document) {
  global.document = document;
  delete require.cache[SCRIPT];
  return require(SCRIPT);
}

// A searchable field as the server renders it: the combobox (a filter input
// over a listbox) beside the select primitive's wrapper `<span>`, which holds
// the native `<select>` and the chevron.
function searchableField() {
  const options = [
    { value: '', textContent: '-- Select --' },
    { value: 'pk-ada', textContent: 'Ada Author' },
    { value: 'pk-alan', textContent: 'Alan Author' },
  ];
  const rows = options.slice(1).map((option) => {
    const attrs = { role: 'option', 'aria-selected': 'false' };
    return {
      dataset: { value: option.value },
      textContent: option.textContent,
      getAttribute: (name) => (name in attrs ? attrs[name] : null),
      setAttribute: (name, value) => {
        attrs[name] = value;
      },
      scrollIntoView() {},
    };
  });

  const combo = {};
  const wrap = {};
  const filter = {
    value: '',
    closest: (selector) =>
      selector === '[data-options-filter]' ? filter
        : selector === '[data-options-combobox]' ? combo
          : selector === '[data-select-filterable]' ? wrap
            : null,
  };
  const list = {
    hidden: false,
    querySelector: (selector) => {
      if (selector === '[role="option"][aria-selected="true"]') {
        return rows.find((row) => row.getAttribute('aria-selected') === 'true') || null;
      }
      return selector === '[role="option"]' ? rows[0] || null : null;
    },
    querySelectorAll: (selector) => (selector === '[role="option"]' ? rows : []),
    replaceChildren: () => rows.splice(0),
    appendChild: (row) => rows.push(row),
  };
  combo.querySelector = (selector) =>
    selector === '[data-options-filter]' ? filter
      : selector === '[data-options-list]' ? list
        : null;
  const select = {
    value: '',
    options,
    events: [],
    dispatchEvent(event) {
      this.events.push(event);
      return true;
    },
  };
  // The primitive's wrapper `<span>`, which draws the chevron.
  const control = { hidden: false };
  select.parentElement = control;
  wrap.querySelector = (selector) => (selector === 'select' ? select : null);
  wrap.getAttribute = () => null;
  wrap.dataset = {};
  rows.forEach((row) => {
    row.closest = (selector) =>
      selector === '[data-options-list] [role="option"]' ? row
        : selector === '[data-options-combobox]' ? combo
          : selector === '[data-select-filterable]' ? wrap
            : null;
  });
  return { combo, wrap, filter, list, select, control, rows };
}

// The matching cases are pure; a document with no fields is enough to load the
// script.
const { matchingOptions, MAX_LIST_ITEMS, shouldHideNativeSelect } = load(standInDocument([]));

const PLACEHOLDER = { value: '', label: '-- Select --', selected: false };
const ada = { value: 'pk-ada', label: 'Ada Author', selected: false };
const alan = { value: 'pk-alan', label: 'Alan Author', selected: false };
const june = { value: 'pk-june', label: 'June Writer', selected: false };
const options = [PLACEHOLDER, ada, alan, june];

const labels = (needle) => matchingOptions(options, needle, MAX_LIST_ITEMS).map((r) => r.label);

test('an empty needle offers everything and the placeholder', () => {
  assert.deepEqual(labels(''), ['-- Select --', 'Ada Author', 'Alan Author', 'June Writer']);
});

test('a needle narrows by label substring', () => {
  assert.deepEqual(labels('ada'), ['Ada Author']);
  assert.deepEqual(labels('author'), ['Ada Author', 'Alan Author']);
});

test('matching is case-insensitive, in both directions', () => {
  assert.deepEqual(labels('ADA'), ['Ada Author']);
  assert.deepEqual(labels('wRiTeR'), ['June Writer']);
});

test('the needle is trimmed before matching', () => {
  assert.deepEqual(labels('  june  '), ['June Writer']);
});

test('the placeholder is offered only while the needle is empty', () => {
  // While filtering it is not a match for anything, so it must not sit at the
  // top of a narrowed list pretending to be one.
  assert.equal(matchingOptions(options, 'ada', MAX_LIST_ITEMS)[0].label, 'Ada Author');
  assert.ok(!labels('ada').includes('-- Select --'));
});

test('the placeholder never counts against the cap', () => {
  // Otherwise a full page of options would collapse to limit - 1 real choices,
  // and the row that fell off would be invisible rather than merely last.
  const many = [PLACEHOLDER];
  for (let i = 0; i < 10; i += 1) {
    many.push({ value: `pk-${i}`, label: `Option ${i}`, selected: false });
  }
  const rows = matchingOptions(many, '', 3);
  assert.equal(rows.length, 4, 'the placeholder plus the cap');
  assert.equal(rows[0].label, '-- Select --');
  assert.deepEqual(rows.slice(1).map((r) => r.label), ['Option 0', 'Option 1', 'Option 2']);
});

test('a needle narrower than the cap still returns every match', () => {
  assert.deepEqual(labels('author'), ['Ada Author', 'Alan Author']);
});

test('selection state rides along, so the list can mark the current choice', () => {
  const chosen = [{ ...PLACEHOLDER }, { ...ada, selected: true }];
  const rows = matchingOptions(chosen, '', MAX_LIST_ITEMS);
  assert.equal(rows.find((r) => r.value === 'pk-ada').selected, true);
  assert.equal(rows.find((r) => r.value === '').selected, false);
});

test('no match yields an empty list, which the caller reports as such', () => {
  assert.deepEqual(labels('zzz'), []);
});

// --- GH #236: the native select behind the combobox --------------------------

test('the native select is hidden only when the combobox over it is wired', () => {
  const wired = { combo: {}, wrap: {}, filter: {}, list: {}, select: {} };
  assert.equal(shouldHideNativeSelect(wired), true);
  // A field the script cannot drive keeps the only control it has: no
  // combobox at all is a plain, non-searchable select.
  for (const part of ['combo', 'filter', 'list', 'select']) {
    assert.equal(
      shouldHideNativeSelect({ ...wired, [part]: null }),
      false,
      `must not hide without ${part}`,
    );
  }
});

test('a wired field hides the replaced control, not the field', () => {
  const world = searchableField();
  load(standInDocument([world.filter]));
  // The chevron rides in the select primitive's wrapper, so that wrapper is
  // what leaves the display; hiding the `<select>` alone would leave it.
  assert.equal(world.control.hidden, true, 'the replaced control leaves the display');
  assert.notEqual(world.wrap.hidden, true, 'the field wrapper keeps the combobox');
  assert.equal(world.select.options.length, 3, 'the select stays the value carrier');
});

test('a plain select is left visible', () => {
  const world = searchableField();
  load(standInDocument([]));
  assert.notEqual(world.control.hidden, true, 'nothing replaces a select with no combobox');
});

// --- GH #237: one wiring pass -------------------------------------------------

test('every document listener is registered once', () => {
  const world = searchableField();
  const document = standInDocument([world.filter]);
  load(document);
  assert.deepEqual(
    document.types().sort().map((type) => `${type}:${document.listeners(type).length}`),
    ['change:1', 'click:1', 'focusin:1', 'focusout:1', 'input:1', 'keydown:1', 'mousedown:1'],
    'one listener per type, so one wiring block',
  );
});

test('an activation dispatches one change', () => {
  const world = searchableField();
  const document = standInDocument([world.filter]);
  load(document);
  // Picking a row: the mousedown handler writes the choice onto the select and
  // announces it. A second wiring block would run it twice.
  document.listeners('mousedown').forEach((handler) => {
    handler({ target: world.rows[0], preventDefault() {} });
  });
  assert.equal(world.select.events.length, 1, 'one change per activation');
  assert.equal(world.select.events[0].type, 'change');
  assert.equal(world.select.value, 'pk-ada');
});

test('an arrow key advances one row', () => {
  const world = searchableField();
  const document = standInDocument([world.filter]);
  load(document);
  // No row starts selected, so `activeItem` is the first: one ArrowDown lands
  // on the second. A second wiring block re-reads the live `aria-selected` and
  // advances again, to the third.
  document.listeners('keydown').forEach((handler) => {
    handler({ key: 'ArrowDown', target: world.filter, preventDefault() {} });
  });
  const selected = world.rows.filter((row) => row.getAttribute('aria-selected') === 'true');
  assert.equal(selected.length, 1, 'one row per arrow key');
  assert.equal(selected[0].dataset.value, 'pk-alan', 'the row after the first');
});
