// Unit test for the one pure decision in `selects.js` (GH #184).
//
// There is no JS test runner in this workspace — the assets are plain browser
// scripts loaded through `asset!` — so this runs on Node's built-in runner and
// reaches the function through the guarded `module.exports` at the bottom of
// the script:
//
//     cargo test -p argentum-ui            # renders and Rust-side assertions
//     node --test crates/argentum-ui/assets/selects.test.js
//
// What it protects: the reason GH #184 was filed is that a filter appeared to
// work and did not. The matching rule is therefore pinned directly, including
// the two easy mistakes — case sensitivity and letting the placeholder eat the
// cap.

const test = require('node:test');
const assert = require('node:assert/strict');

const { matchingOptions, MAX_LIST_ITEMS } = require('./selects.js');

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
