// Unit tests for the toast lifetime in `notifications.js` (GH #293).
//
// There is no JS test runner in this workspace — the assets are plain browser
// scripts loaded through `asset!` — so this runs on Node's built-in runner and
// reaches the lifecycle through the guarded `module.exports` at the bottom of
// the script:
//
//     node --test crates/argentum-ui/assets/notifications.test.js
//
// What the cases protect: the toast lifetime is one timer, and the countdown
// runs only while no pause source holds it. Hover and focus are independent,
// so a resume from one must not arm a timer the other still pauses, and the
// remaining time survives any sequence of pauses and resumes. The clock below
// is manual: `Date.now` and `window.setTimeout` are the script's only time
// sources, and the case decides exactly when each expires.

const test = require('node:test');
const assert = require('node:assert/strict');

const SCRIPT = require.resolve('./notifications.js');
const { TOAST_LIFETIME, armToast } = require(SCRIPT);

// A toast as the shell renders it: the dataset `armToast` writes, the event
// listeners it installs, and the removal it drives.
function toast() {
  const listeners = new Map();
  return {
    dataset: {},
    offsetHeight: 0,
    removed: false,
    addEventListener(type, handler) {
      if (!listeners.has(type)) listeners.set(type, []);
      listeners.get(type).push(handler);
    },
    // Fire one event, the way the browser would.
    fire(type, event = {}) {
      (listeners.get(type) || []).forEach((handler) => handler(event));
    },
    // `focusout` asks whether focus landed inside the toast.
    contains: () => false,
    remove() {
      this.removed = true;
    },
  };
}

// The script's clock: `window.setTimeout`/`clearTimeout` back onto a manual
// queue, and `Date.now` onto the same counter, so a case advances time and
// observes exactly which timer fired.
function withClock(run) {
  let now = 0;
  let nextId = 1;
  const timers = new Map();
  const clock = {
    now: () => now,
    pending: () => timers.size,
    tick(ms) {
      now += ms;
      // Fire every timer that came due, including ones a callback schedules.
      let fired = true;
      while (fired) {
        fired = false;
        for (const [id, timer] of Array.from(timers)) {
          if (timer.at > now) continue;
          timers.delete(id);
          timer.run();
          fired = true;
        }
      }
    },
  };
  const realNow = Date.now;
  const realWindow = global.window;
  Date.now = () => clock.now();
  global.window = {
    setTimeout: (fn, ms) => {
      const id = nextId++;
      timers.set(id, { run: fn, at: now + ms });
      return id;
    },
    clearTimeout: (id) => {
      timers.delete(id);
    },
  };
  try {
    return run(clock);
  } finally {
    Date.now = realNow;
    global.window = realWindow;
  }
}

test('a toast is dismissed after the lifetime', () => {
  withClock((clock) => {
    const el = toast();
    armToast(el);
    clock.tick(TOAST_LIFETIME - 1);
    assert.equal(el.dataset.removed, undefined, 'the lifetime is not up yet');
    clock.tick(1);
    assert.equal(el.dataset.removed, 'true', 'the lifetime dismisses the toast');
  });
});

test('hover then leave resumes with the remaining time', () => {
  withClock((clock) => {
    const el = toast();
    armToast(el);
    clock.tick(1000);
    el.fire('mouseenter');
    // Hovered, the countdown stops: five lifetimes of wall time pass and the
    // toast stays.
    clock.tick(TOAST_LIFETIME * 5);
    assert.equal(el.dataset.removed, undefined, 'a hovered toast stays');
    el.fire('mouseleave');
    // The 3s left, not a fresh lifetime: just short of them the toast is up.
    clock.tick(TOAST_LIFETIME - 1000 - 1);
    assert.equal(el.dataset.removed, undefined, 'the remaining 3s are not up');
    clock.tick(1);
    assert.equal(el.dataset.removed, 'true', 'the remaining 3s dismiss it');
  });
});

test('focus while hovered does not resume on mouseleave', () => {
  withClock((clock) => {
    const el = toast();
    armToast(el);
    el.fire('mouseenter');
    el.fire('focusin');
    clock.tick(TOAST_LIFETIME * 2);
    // Hover leaves, but the reader is still in the toast: the resume must not
    // arm a timer the focus still pauses.
    el.fire('mouseleave');
    assert.equal(clock.pending(), 0, 'no timer runs while the toast has focus');
    clock.tick(TOAST_LIFETIME * 2);
    assert.equal(el.dataset.removed, undefined, 'a focused toast stays');
    el.fire('focusout', { relatedTarget: null });
    clock.tick(TOAST_LIFETIME);
    assert.equal(el.dataset.removed, 'true', 'leaving every pause source resumes');
  });
});

test('the timer never fires while a pause source is active', () => {
  withClock((clock) => {
    const el = toast();
    armToast(el);
    clock.tick(1);
    el.fire('focusin');
    clock.tick(TOAST_LIFETIME * 10);
    assert.equal(el.dataset.removed, undefined, 'a focused toast must not be dismissed');
    el.fire('mouseenter');
    clock.tick(TOAST_LIFETIME * 10);
    assert.equal(el.dataset.removed, undefined, 'a hovered and focused toast neither');
  });
});

test('a second pause from the same source does not restart the countdown', () => {
  withClock((clock) => {
    const el = toast();
    armToast(el);
    clock.tick(1000);
    el.fire('mouseenter');
    clock.tick(500);
    // A repeat entrance (a nested element's event) must not add a second
    // pause it would take two leaves to lift.
    el.fire('mouseenter');
    el.fire('mouseleave');
    assert.equal(clock.pending(), 1, 'one resume arms one timer');
    clock.tick(TOAST_LIFETIME - 1000 - 1);
    assert.equal(el.dataset.removed, undefined, 'still the original remaining time');
    clock.tick(1);
    assert.equal(el.dataset.removed, 'true');
  });
});
