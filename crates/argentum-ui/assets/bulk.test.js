// Unit test for the two pure decisions in `bulk.js` (GH #235).
//
// There is no JS test runner in this workspace — the assets are plain browser
// scripts loaded through `asset!` — so this runs on Node's built-in runner and
// reaches the functions through the guarded `module.exports` at the bottom of
// the script:
//
//     node --test crates/argentum-ui/assets/bulk.test.js
//
// What it protects: a row the resource's per-record policy denies delete renders
// its bulk checkbox `disabled`, and a disabled box must never reach the hidden
// `ids` transport. The handler's check is all-or-nothing, so one refused key
// would turn select-all over a whole page into a 403 with zero deletions — the
// bug GH #235 was filed for. The tri-state header has the same rule: a denied
// row is not a row it can speak for, or a page whose every allowed row is
// checked would still read "partial".

const test = require('node:test');
const assert = require('node:assert/strict');

const { headerState, selectionKeys, wireFrom, wireOf } = require('./bulk.js');

// A row checkbox, as the DOM hands it over: value + checked + disabled.
const box = (value, { checked = false, disabled = false } = {}) => ({
  value,
  checked,
  disabled,
});

test('a disabled box never enters the wire, checked or not', () => {
  const boxes = [
    box('ada', { checked: true }),
    box('ken', { checked: true, disabled: true }),
    box('grace'),
  ];
  assert.deepEqual(selectionKeys(boxes, ''), ['ada']);
});

test('a stale wire entry for a now-denied row is dropped', () => {
  // The key was selected before the page re-rendered the row as denied (a
  // filter change or a shard swap reuses the element): the page owns that key
  // now, and its box says no.
  assert.deepEqual(selectionKeys([box('ken', { disabled: true })], ',ken,'), []);
});

test('keys selected on other pages survive the page-local read', () => {
  // Those rows are not in the DOM at all, so only the transport knows them.
  const boxes = [box('ada', { checked: true }), box('ken', { disabled: true })];
  assert.deepEqual(selectionKeys(boxes, ',grace,'), ['grace', 'ada']);
});

test('a key on this page is decided by this page, never duplicated', () => {
  const boxes = [box('ada', { checked: true })];
  assert.deepEqual(selectionKeys(boxes, ',ada,'), ['ada']);
});

test('the header reads "all" when every selectable row is checked', () => {
  // The denied row sits in the page unchecked; it must not make the header
  // report a partial selection.
  const selectable = [box('ada', { checked: true }), box('grace', { checked: true })];
  assert.deepEqual(headerState(selectable), { checked: true, indeterminate: false });
});

test('the header reads "partial" on a partial selection', () => {
  const selectable = [box('ada', { checked: true }), box('grace')];
  assert.deepEqual(headerState(selectable), { checked: false, indeterminate: true });
});

test('a page of only denied rows offers nothing to select', () => {
  // `boxesIn` yields the selectable boxes, so this is what a page whose every
  // row is refused hands the header: neither checked nor indeterminate.
  assert.deepEqual(headerState([]), { checked: false, indeterminate: false });
});

test('the wire is comma-delimited on both ends, so membership is exact', () => {
  assert.equal(wireFrom(['ab']), ',ab,');
  assert.equal(wireFrom([]), '');
  assert.deepEqual(wireOf(',ab,'), ['ab']);
  assert.deepEqual(wireOf(''), []);
  assert.ok(!wireOf(',ab,').includes('b'), 'a substring is not a member');
});
