// Unit tests for `selects.js`.
//
// There is no JS test runner in this workspace — the assets are plain browser
// scripts loaded through `asset!` — so this runs on Node's built-in runner and
// reaches the script through the guarded `module.exports` at the bottom of the
// script:
//
//     cargo test -p tablo-ui            # renders and Rust-side assertions
//     node --test crates/tablo-ui/assets/selects.test.js
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

const { listenerDocument } = require('./test-dom');

const SCRIPT = require.resolve('./selects.js');

// --- a document stand-in -----------------------------------------------------

// `selects.js` is a plain browser script: `install()` reads `document` from the
// global scope and every handler is document-delegated, so the stand-in has to
// be in place before the script is required. It is only as wide as the script
// needs: each node answers the selectors `partsOf` and the listbox read.
function standInDocument(filters) {
  return listenerDocument({
    activeElement: null,
    documentElement: {},
    querySelectorAll: (selector) =>
      selector === '[data-options-filter]' ? filters : [],
    // The `<li>`s `renderList` builds.
    createElement: () => listItem(),
  });
}

// A created `<li>`, as `renderList` fills it in.
function listItem() {
  const attrs = {};
  return {
    dataset: {},
    textContent: '',
    className: '',
    id: '',
    getAttribute: (name) => (name in attrs ? attrs[name] : null),
    setAttribute: (name, value) => {
      attrs[name] = value;
    },
    removeAttribute: (name) => {
      delete attrs[name];
    },
    scrollIntoView() {},
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
// the native `<select>` and the chevron. `server` names the field the
// overflowed relationship fetches options for.
function searchableField({ server = null } = {}) {
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
  const wrapAttrs = server
    ? { 'data-options-server': 'true', 'data-options-field': server }
    : {};
  const filterAttrs = {};
  const filter = {
    value: '',
    closest: (selector) =>
      selector === '[data-options-filter]' ? filter
        : selector === '[data-options-combobox]' ? combo
          : selector === '[data-select-filterable]' ? wrap
            : null,
    getAttribute: (name) => (name in filterAttrs ? filterAttrs[name] : null),
    setAttribute: (name, value) => {
      filterAttrs[name] = value;
    },
    removeAttribute: (name) => {
      delete filterAttrs[name];
    },
  };
  const list = {
    id: 'name-options-list',
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
    required: false,
    events: [],
    querySelectorAll: (selector) => (selector === 'option' ? options : []),
    dispatchEvent(event) {
      this.events.push(event);
      return true;
    },
  };
  // The primitive's wrapper `<span>`, which draws the chevron.
  const control = { hidden: false };
  select.parentElement = control;
  wrap.querySelector = (selector) => (selector === 'select' ? select : null);
  wrap.getAttribute = (name) => (name in wrapAttrs ? wrapAttrs[name] : null);
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

// A non-searchable field: the field wrapper, the select primitive's wrapper
// `<span>`, and a native `<select>` with no combobox over it.
function plainField() {
  const select = { value: '', required: true };
  const control = { hidden: false };
  select.parentElement = control;
  const wrap = { querySelector: (selector) => (selector === 'select' ? select : null) };
  return { wrap, select, control };
}

// The matching cases are pure; a document with no fields is enough to load the
// script.
const { matchingOptions, MAX_LIST_ITEMS, preservedOption, shouldHideNativeSelect } =
  load(standInDocument([]));

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

test('hiding drops the required attribute the browser would block on', () => {
  const world = searchableField();
  world.select.required = true;
  load(standInDocument([world.filter]));
  // Constraint validation still runs for a `display: none` control, and a
  // failed one cannot take focus, so the browser refuses the submit before any
  // `submit` handler sees it.
  assert.equal(world.select.required, false, 'the unfocusable control is not required');
  assert.equal(
    world.filter.getAttribute('aria-required'),
    'true',
    'the combobox carries the field requiredness',
  );
});

test('a plain select keeps its control and its required attribute', () => {
  // A non-searchable field shares the document with the wired one, so the pass
  // really runs and really leaves it alone.
  const searchable = searchableField();
  const plain = plainField();
  load(standInDocument([searchable.filter]));
  assert.equal(searchable.control.hidden, true, 'the wired field hides its control');
  assert.notEqual(plain.control.hidden, true, 'nothing replaces a select with no combobox');
  assert.equal(plain.select.required, true, 'native validation stays on the visible control');
});

test('a swapped-in field is hidden again', () => {
  const world = searchableField();
  const document = standInDocument([world.filter]);
  const observers = [];
  global.MutationObserver = class {
    constructor(callback) {
      this.callback = callback;
      observers.push(this);
    }

    observe() {}
  };
  try {
    load(document);
    assert.equal(observers.length, 1, 'the script watches for swapped markup');
    // The swap replaces the field, so the server-rendered control is visible
    // again until the observer replays the hide pass on the new nodes.
    world.control.hidden = false;
    observers[0].callback([
      { addedNodes: [{ nodeType: 1, querySelectorAll: () => [world.filter] }] },
    ]);
    assert.equal(world.control.hidden, true, 'the swapped field is hidden again');
  } finally {
    delete global.MutationObserver;
  }
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
  // advances again, which wraps to the first.
  document.listeners('keydown').forEach((handler) => {
    handler({ key: 'ArrowDown', target: world.filter, preventDefault() {} });
  });
  const selected = world.rows.filter((row) => row.getAttribute('aria-selected') === 'true');
  assert.equal(selected.length, 1, 'one row per arrow key');
  assert.equal(selected[0].dataset.value, 'pk-alan', 'the row after the first');
});

// --- GH #293: the current option, the combobox ARIA, and Enter ---------------

test('an edit form shows the current option label in the box', () => {
  // The native select is hidden and the filter input is the box left in its
  // place, so an edit form opens on the record's stored label, not empty.
  const world = searchableField();
  world.select.value = 'pk-ada';
  load(standInDocument([world.filter]));
  assert.equal(world.filter.value, 'Ada Author');
});

test('a field with no current option leaves the box empty', () => {
  // The placeholder is the empty value: it is not a choice to display.
  const world = searchableField();
  load(standInDocument([world.filter]));
  assert.equal(world.filter.value, '');
});

test('a swap keeps the current option under its own label', () => {
  // The server answers the needle, not the selection; the label exists only in
  // the option the swap drops, so re-attach it by label rather than showing the
  // primary key as its own.
  assert.equal(
    preservedOption('pk-ada', 'Ada Author', '<option value="pk-ken">Ken</option>'),
    '<option value="pk-ada" selected>Ada Author</option>',
  );
});

test('a swap that already carries the current option does not duplicate it', () => {
  assert.equal(
    preservedOption('pk-ada', 'Ada Author', '<option value="pk-ada">Ada Author</option>'),
    null,
  );
});

test('an empty selection preserves nothing', () => {
  assert.equal(preservedOption('', 'Ada Author', ''), null);
});

test('a record label cannot break out of the option markup', () => {
  assert.equal(
    preservedOption('pk-1', 'A "quoted" <name>', ''),
    '<option value="pk-1" selected>A &quot;quoted&quot; &lt;name&gt;</option>',
  );
});

test('opening the combobox expands it and names its active row', () => {
  const world = searchableField();
  const document = standInDocument([world.filter]);
  load(document);
  document.listeners('focusin').forEach((handler) => handler({ target: world.filter }));
  assert.equal(world.list.hidden, false, 'the list is showing');
  assert.equal(world.filter.getAttribute('aria-expanded'), 'true');
  assert.equal(
    world.filter.getAttribute('aria-activedescendant'),
    'name-options-list-option-0',
    'the first row is the active descendant',
  );
});

test('an arrow key moves the active descendant with the selection', () => {
  const world = searchableField();
  const document = standInDocument([world.filter]);
  load(document);
  document.listeners('focusin').forEach((handler) => handler({ target: world.filter }));
  document.listeners('keydown').forEach((handler) => {
    handler({ key: 'ArrowDown', target: world.filter, preventDefault() {} });
  });
  assert.equal(
    world.filter.getAttribute('aria-activedescendant'),
    'name-options-list-option-1',
    'the active row follows the arrow',
  );
});

test('closing the combobox collapses it and clears the active row', () => {
  const world = searchableField();
  const document = standInDocument([world.filter]);
  load(document);
  document.listeners('focusin').forEach((handler) => handler({ target: world.filter }));
  document.listeners('keydown').forEach((handler) => {
    handler({ key: 'Escape', target: world.filter, preventDefault() {} });
  });
  assert.equal(world.list.hidden, true);
  assert.equal(world.filter.getAttribute('aria-expanded'), 'false');
  assert.equal(world.filter.getAttribute('aria-activedescendant'), null);
});

test('Enter while the combobox has focus never submits the form', () => {
  // The list is collapsed: the keystroke is still the combobox's, not the
  // form's implicit submit.
  const world = searchableField();
  const document = standInDocument([world.filter]);
  load(document);
  world.list.hidden = true;
  const event = {
    key: 'Enter',
    target: world.filter,
    prevented: false,
    preventDefault() {
      this.prevented = true;
    },
  };
  document.listeners('keydown').forEach((handler) => handler(event));
  assert.equal(event.prevented, true, 'the form must not submit');
  assert.equal(world.select.events.length, 0, 'nothing was chosen');
});

test('Enter while the server is searching does not submit', () => {
  // The status row ("Searching…") offers no option to pick, and the keystroke
  // must not submit the record the reader is editing.
  const world = searchableField({ server: 'author_id' });
  const document = standInDocument([world.filter]);
  const realSetTimeout = global.setTimeout;
  const realClearTimeout = global.clearTimeout;
  global.setTimeout = () => 0;
  global.clearTimeout = () => {};
  try {
    load(document);
    document.listeners('input').forEach((handler) => handler({ target: world.filter }));
    assert.equal(world.list.hidden, false, 'the status row shows the search');
    const event = {
      key: 'Enter',
      target: world.filter,
      prevented: false,
      preventDefault() {
        this.prevented = true;
      },
    };
    document.listeners('keydown').forEach((handler) => handler(event));
    assert.equal(event.prevented, true, 'the form must not submit');
    assert.equal(
      world.filter.getAttribute('aria-expanded'),
      'true',
      'the status popup is expanded',
    );
    assert.equal(
      world.filter.getAttribute('aria-activedescendant'),
      null,
      'a status line has no active row',
    );
  } finally {
    global.setTimeout = realSetTimeout;
    global.clearTimeout = realClearTimeout;
  }
});
