// Unit tests for `media.js`.
//
// There is no JS test runner in this workspace — the assets are plain browser
// scripts loaded through `asset!` — so this runs on Node's built-in runner and
// reaches the functions through the guarded `module.exports` at the bottom of
// the script:
//
//     node --test examples/showcase/assets/media.test.js
//
// What the cases protect:
// * the preview decision: an `image/*` file draws a thumbnail, anything else
//   draws its name, and nothing selected empties the region;
// * the clear control clears the file input and the preview **without**
//   resetting the form: the owner the user picked has to survive a file clear,
//   which is why the script cancels the reset the markup's button would
//   otherwise perform. With the script off that reset is the no-JS fallback,
//   and ADR-0021 records both paths;
// * the object URL is revoked when the preview is replaced or cleared, so a
//   page left open does not pin the file's bytes.

const test = require('node:test');
const assert = require('node:assert/strict');

const SCRIPT = require.resolve('./media.js');

const { listenerDocument } = require('../../../crates/argentum-ui/assets/test-dom.js');

// --- browser stand-ins -------------------------------------------------------

// The widget as the page renders it: a form holding the file input, the preview
// region, and the clear control, each reachable the way the script reaches it.
function widget() {
  const region = {
    hidden: true,
    dataset: {},
    children: [],
    appendChild(node) {
      this.children.push(node);
    },
    replaceChildren() {
      this.children.length = 0;
    },
  };
  const input = {
    files: [],
    value: '',
    matches: (selector) => selector === '[data-media-file]',
    form: null,
  };
  const control = { form: null };
  const form = {
    querySelector: (selector) =>
      selector === '[data-media-preview]' ? region
        : selector === '[data-media-file]' ? input
          : null,
  };
  input.form = form;
  control.form = form;
  return { region, input, control };
}

// A document that records the listeners `install()` registers, so a case can
// fire them the way a browser does.
function standInDocument() {
  return listenerDocument({
    createElement(tag) {
      return { tag };
    },
    createTextNode(text) {
      return { text };
    },
  });
}

// Load a fresh copy of the script against `document`: a fresh copy re-runs
// `install()`, so each case gets its own listener set.
function load(document) {
  global.document = document;
  delete require.cache[SCRIPT];
  return require(SCRIPT);
}

// Every listener for `type`, in registration order: firing them all is what a
// browser does for one event.
function fire(document, type, event) {
  document.listeners(type).forEach((handler) => handler(event));
}

// The object-URL half of the browser, recorded instead of allocated.
function stubObjectUrls() {
  const revoked = [];
  const previous = global.URL;
  global.URL = {
    createObjectURL: (file) => `blob:${file.name}`,
    revokeObjectURL: (url) => revoked.push(url),
  };
  return {
    revoked,
    restore() {
      global.URL = previous;
    },
  };
}

// --- the preview decision ----------------------------------------------------

test('an image previews as a thumbnail and anything else as its name', () => {
  const urls = stubObjectUrls();
  try {
    const { region, input } = widget();
    const { showPreview } = load(standInDocument());

    input.files = [{ name: 'cover.png', type: 'image/png' }];
    showPreview(input);
    assert.equal(region.hidden, false, 'a chosen file shows a preview');
    assert.deepEqual(
      region.children.map((node) => node.tag),
      ['img'],
      'an image previews as an <img>',
    );
    assert.equal(region.children[0].src, 'blob:cover.png', 'the thumbnail is the object URL');
    assert.equal(region.children[0].alt, 'cover.png', 'the thumbnail carries the file name');

    input.files = [{ name: 'notes.txt', type: 'text/plain' }];
    showPreview(input);
    assert.ok(
      region.children.every((node) => node.tag !== 'img'),
      'a non-image is never drawn as a thumbnail',
    );
    assert.deepEqual(
      region.children.map((node) => node.text),
      ['notes.txt'],
      'a non-image previews as its name',
    );

    input.files = [];
    showPreview(input);
    assert.equal(region.hidden, true, 'nothing chosen hides the preview');
    assert.deepEqual(region.children, [], 'and empties it');
  } finally {
    urls.restore();
  }
});

test('replacing a preview revokes the object URL it was showing', () => {
  const urls = stubObjectUrls();
  try {
    const { region, input } = widget();
    const { showPreview } = load(standInDocument());

    input.files = [{ name: 'first.png', type: 'image/png' }];
    showPreview(input);
    input.files = [{ name: 'second.png', type: 'image/png' }];
    showPreview(input);

    assert.deepEqual(urls.revoked, ['blob:first.png'], 'the replaced URL is revoked');
    assert.equal(region.dataset.objectUrl, 'blob:second.png');
  } finally {
    urls.restore();
  }
});

// --- the clear control -------------------------------------------------------

test('the clear control empties the file and cancels the form reset', () => {
  const urls = stubObjectUrls();
  try {
    const { region, input, control } = widget();
    const document = standInDocument();
    const { showPreview } = load(document);
    input.files = [{ name: 'cover.png', type: 'image/png' }];
    input.value = 'C:\\fakepath\\cover.png';
    showPreview(input);
    assert.equal(region.hidden, false);

    let cancelled = 0;
    fire(document, 'click', {
      target: {
        closest: (selector) => (selector === '[data-media-clear]' ? control : null),
      },
      preventDefault: () => (cancelled += 1),
    });

    assert.equal(
      cancelled,
      1,
      'the reset is cancelled: it would also drop the owner the user picked',
    );
    assert.equal(input.value, '', 'the file input is emptied by the script');
    assert.equal(region.hidden, true, 'the preview goes');
    assert.deepEqual(region.children, []);
    assert.deepEqual(urls.revoked, ['blob:cover.png'], 'and its object URL is revoked');
  } finally {
    urls.restore();
  }
});

test('a clear control with no file input in reach keeps the browser reset', () => {
  const urls = stubObjectUrls();
  try {
    const document = standInDocument();
    load(document);
    // A form the widget cannot find its input in: the click must fall through
    // to the reset the markup declared, not be swallowed.
    const control = { form: { querySelector: () => null } };

    let cancelled = 0;
    fire(document, 'click', {
      target: {
        closest: (selector) => (selector === '[data-media-clear]' ? control : null),
      },
      preventDefault: () => (cancelled += 1),
    });

    assert.equal(cancelled, 0, 'the browser still resets the form');
  } finally {
    urls.restore();
  }
});

// --- the delegation ----------------------------------------------------------

test('the widget is wired from the document, not from the markup', () => {
  const urls = stubObjectUrls();
  try {
    const { region, input } = widget();
    const document = standInDocument();
    load(document);

    input.files = [{ name: 'cover.png', type: 'image/png' }];
    fire(document, 'change', { target: input });
    assert.equal(region.hidden, false, 'a change on the file input draws the preview');

    // A control that is not the widget's, and a click that is not the clear
    // control: neither touches the preview.
    fire(document, 'change', { target: { matches: () => false } });
    fire(document, 'click', { target: { closest: () => null } });
    assert.equal(region.hidden, false, 'unrelated events leave the preview alone');
  } finally {
    urls.restore();
  }
});
