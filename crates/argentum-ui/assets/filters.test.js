// Unit test for the filter transport `filters.js` composes (GH #93, GH #294).
//
// The assets are plain browser scripts loaded through `asset!`, so this runs on
// Node's built-in test runner and reaches the script through the guarded
// `module.exports` at the bottom of the script:
//
//     node --test crates/argentum-ui/assets/filters.test.js
//
// What it protects: a filter value the transport's own grammar uses cannot
// change the meaning of the transport. The server splits it on `,`, then on the
// first `:`, and percent-decodes each half (`parse_filters_param` /
// `decode_filter_component` in `crates/argentum-core/src/resource/state.rs`),
// so `Smith, John` composed verbatim is two segments. `filters.js` escapes
// `%`, `:`, `,` in both halves before joining them.
//
// The encoder and the parser below are transcriptions of the Rust pair, because
// Node cannot call the Rust functions. `filters.js`'s `encodeFilterComponent`
// is the half under test: the first assertion of every case checks it against
// the transcription. The server half of the boundary is pinned on the Rust
// side, by `client_transport_parses_to_the_client_value` in `state.rs`, which
// parses the literal this script composes.

const test = require('node:test');
const assert = require('node:assert/strict');

const SCRIPT = require.resolve('./filters.js');

// `filters.js` is a plain browser script: it registers its listeners on
// `document` at load time, so a stand-in has to be in place before it is
// required (the same pattern as `selects.test.js`).
global.document = { addEventListener() {} };

const { composeFilters, encodeFilterComponent } = require(SCRIPT);

// --- the server's half, transcribed from `state.rs` -------------------------

// Mirrors `encode_filter_component`: `%` first, then `:`, then `,`.
function rustEncode(s) {
  return s.replace(/%/g, '%25').replace(/:/g, '%3A').replace(/,/g, '%2C');
}

// Mirrors `decode_filter_component`: one pass, case-insensitive hex. The order
// matters — `%25` last, or `%253A` would decode twice.
function rustDecode(s) {
  return s
    .replace(/%2C/g, ',')
    .replace(/%2c/g, ',')
    .replace(/%3A/g, ':')
    .replace(/%3a/g, ':')
    .replace(/%25/g, '%');
}

// Mirrors `parse_filters_param`: split on `,`, first `:` splits the pair, trim
// each half, decode. A blank segment is skipped; a colon-less or empty-half
// segment is malformed; a duplicated key keeps the first value.
function parseFiltersParam(raw) {
  const filters = new Map();
  const malformed = [];
  for (const rawPart of raw.split(',')) {
    const part = rawPart.trim();
    if (!part) continue;
    const colon = part.indexOf(':');
    if (colon < 0) {
      malformed.push(part);
      continue;
    }
    const key = rustDecode(part.slice(0, colon).trim());
    const value = rustDecode(part.slice(colon + 1).trim());
    if (!key || !value) malformed.push(part);
    else if (!filters.has(key)) filters.set(key, value);
  }
  return { filters, malformed };
}

// --- stand-ins --------------------------------------------------------------

// One typed control, as `composeFilters` reads it: the `data-filter-name`
// attribute and the current value.
function control(name, value) {
  return {
    getAttribute: (attr) => (attr === 'data-filter-name' ? name : null),
    value,
  };
}

function formWith(...controls) {
  return { querySelectorAll: () => controls };
}

// --- the value survives the transport ---------------------------------------

test('a value carrying the transport delimiters comes back unchanged', () => {
  const cases = [
    'Smith, John',
    'a:b',
    '100%',
    // An interior space is data, not grammar.
    'Ada Lovelace',
    // Escape order: the escaped text of one value must not be re-escaped.
    '%3A%2C%25',
  ];
  for (const value of cases) {
    assert.equal(
      encodeFilterComponent(value),
      rustEncode(value),
      `filters.js must escape ${JSON.stringify(value)} like the server`,
    );
    const transport = composeFilters(formWith(control('name', value)));
    const { filters, malformed } = parseFiltersParam(transport);
    assert.deepEqual(malformed, [], `${JSON.stringify(value)}: ${transport}`);
    assert.equal(
      filters.get('name'),
      value,
      `value must round-trip through ${transport}`,
    );
  }
});

test('the key is escaped like the value', () => {
  // Field names are identifiers, so this is the boundary case rather than a
  // real control: an unescaped key can split the transport just as a value can.
  const transport = composeFilters(formWith(control('a,b:c', 'x')));
  assert.equal(transport, 'a%2Cb%3Ac:x');
  const { filters, malformed } = parseFiltersParam(transport);
  assert.deepEqual(malformed, []);
  assert.deepEqual([...filters], [['a,b:c', 'x']]);
});

test('a value with surrounding spaces composes trimmed', () => {
  // `composeFilters` trims the value before joining, and
  // `parse_filters_param` trims each half after splitting, so surrounding
  // whitespace is normalized and the value the transport carries is the one
  // the server reads back. (The key is an attribute value, never trimmed.)
  const transport = composeFilters(formWith(control('name', '  Smith, John  ')));
  assert.equal(transport, 'name:Smith%2C John');
  const { filters, malformed } = parseFiltersParam(transport);
  assert.deepEqual(malformed, []);
  assert.equal(filters.get('name'), 'Smith, John');
});

test('the empty value composes the empty transport that clears the filter', () => {
  // The All option is `value=""`: composing must drop the pair rather than
  // send `name:`, which the server would report as malformed.
  const transport = composeFilters(formWith(control('name', '')));
  assert.equal(transport, '');
  const { filters, malformed } = parseFiltersParam(transport);
  assert.deepEqual(malformed, []);
  assert.equal(filters.size, 0);
});

test('several controls compose one comma-joined transport', () => {
  const transport = composeFilters(
    formWith(control('status', 'published'), control('q', 'a,b')),
  );
  assert.equal(transport, 'status:published,q:a%2Cb');
  const { filters, malformed } = parseFiltersParam(transport);
  assert.deepEqual(malformed, []);
  assert.deepEqual(
    [...filters],
    [
      ['status', 'published'],
      ['q', 'a,b'],
    ],
  );
});
