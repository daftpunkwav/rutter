/**
 * @fileoverview Behavioural tests for the injected DOM serializer.
 *
 * Boundary: runs `crates/observe/src/assets/serializer.js` -- the
 * real, shipped script -- inside a minimal DOM (`dom.mjs`) and
 * asserts what the envelope says. The string-shape assertions that
 * used to live in `crates/observe/src/assets.rs` could not tell a
 * working sweep from a dead one; these run the sweep.
 *
 * GC primitive: `WeakRef` is the one thing the fixture substitutes
 * (a real collection needs an engine GC the test cannot schedule).
 * Everything else -- `pruneRefs`, the store, the accessor, the
 * traversal -- is the shipped code.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { ControllableWeakRef, createPage, el, loadScript, makeDocument } from './dom.mjs';

const SERIALIZER_JS = loadScript('crates/observe/src/assets/serializer.js');

/** A page over `body`, sharing `window` with every later snapshot. */
function pageOver(body, window = {}) {
  const page = createPage({
    document: makeDocument({ body }).document,
    window: { innerWidth: 1024, innerHeight: 768, scrollX: 0, scrollY: 0, ...window },
  });
  return page;
}

/** Marks the store entries at `[from, to)` as garbage-collected. */
function collectRange(page, from, to) {
  let seen = 0;
  let marked = 0;
  page.window.__rutterRefStore.reverse.forEach((weak) => {
    assert.ok(
      weak instanceof ControllableWeakRef,
      'the store holds the page-WeakRef entries the script minted'
    );
    if (seen >= from && seen < to) {
      weak.collected = true;
      marked += 1;
    }
    seen += 1;
  });
  return { marked, seen };
}

test('an actionable element carries a ref and a generic one does not', () => {
  const page = pageOver(el('body', {}, [el('button', {}, ['Save']), el('div', {}, ['plain text'])]));
  const envelope = page.run(SERIALIZER_JS);
  const [button, generic] = envelope.root.children;
  assert.equal(button.role, 'button');
  assert.equal(button.name, 'Save');
  assert.match(button.ref, /^e\d+-[a-z0-9]+$/, 'an actionable element is addressable');
  assert.equal(generic.role, 'generic');
  assert.equal(generic.ref, undefined, 'a generic element is not actionable');
});

test('two documents mint disjoint refs even at the same counter', () => {
  // The regression this pins: every document counts its refs from 1,
  // so two snapshotted pages used to hold the same `e17` and, after a
  // tab switch, a stale reference silently addressed the other page's
  // own element. The per-document scope makes the counters collide
  // without the scopes.
  const pageA = pageOver(el('body', {}, [el('button', {}, ['Save'])]));
  const pageB = pageOver(el('body', {}, [el('button', {}, ['Save'])]));
  const refA = pageA.run(SERIALIZER_JS).root.children[0].ref;
  const refB = pageB.run(SERIALIZER_JS).root.children[0].ref;
  assert.match(refA, /^e1-/);
  assert.match(refB, /^e1-/);
  assert.notEqual(refA, refB, 'the scopes keep one page stale ref from hitting the other');
  assert.equal(pageA.run(SERIALIZER_JS).root.children[0].ref, refA, 'scope is stable per document');
});

test('a snapshot below the sweep threshold leaves the store alone', () => {
  // The sweep is budgeted: running it on every snapshot would make
  // every snapshot O(store), which is the cost the threshold avoids.
  const page = pageOver(
    el('body', {}, Array.from({ length: 8 }, (_, i) => el('button', {}, [`b${i}`])))
  );
  page.run(SERIALIZER_JS);
  assert.equal(page.window.__rutterRefStore.reverse.size, 8);
  assert.equal(collectRange(page, 0, 8).marked, 8, 'all eight refs are collectable');

  page.run(SERIALIZER_JS);
  assert.equal(
    page.window.__rutterRefStore.reverse.size,
    8,
    'a small store is not swept: the entries would be gone from the resolver'
  );
});

test('a snapshot sweeps the refs whose elements the page collected', () => {
  // The regression this pins: the reverse store is a strong Map, so
  // without a sweep it grows with every element the page has ever
  // shown -- a long-lived document that re-renders grows the browser
  // process with it, for as long as the tab stays open.
  const page = pageOver(
    el('body', {}, Array.from({ length: 600 }, (_, i) => el('button', {}, [`row ${i}`])))
  );
  page.run(SERIALIZER_JS);
  const store = page.window.__rutterRefStore;
  assert.equal(
    store.reverse.size,
    600,
    'the first snapshot minted one ref per actionable element'
  );
  assert.equal(collectRange(page, 0, 600).marked, 600);

  // A page that re-rendered: every old element is gone, one is new.
  page.window.document.body = el('body', {}, [el('button', {}, ['one survivor'])]);
  page.run(SERIALIZER_JS);

  assert.equal(
    store.reverse.size,
    1,
    'the sweep dropped every collected ref; only the new element remains'
  );
  assert.equal(store.counter, 601, 'a sweep renumbers nothing; the new ref starts at e601');
});

test('the sweep spares the refs whose elements are still alive', () => {
  // Dropping a live element's entry would break the stability the
  // snapshot format promises: the next snapshot must hand the same
  // element the same ref.
  const buttons = Array.from({ length: 600 }, (_, i) => el('button', {}, [`b${i}`]));
  const page = pageOver(el('body', {}, buttons));
  const first = page.run(SERIALIZER_JS);
  const store = page.window.__rutterRefStore;
  assert.equal(collectRange(page, 0, 500).marked, 500, 'the first five hundred went away');

  const envelope = page.run(SERIALIZER_JS);
  assert.equal(store.reverse.size, 100, 'the 500 collected entries are gone, the live 100 stay');
  const rendered = [...envelope.root.children];
  assert.equal(rendered.length, 600, 'the same document is still fully rendered');
  assert.equal(
    rendered[500].ref,
    first.root.children[500].ref,
    'a live element keeps the ref it was first given'
  );
  assert.equal(rendered[599].ref, first.root.children[599].ref, 'and so does the last');
});

test('a store with no usable shape is left alone instead of throwing', () => {
  // The script promises never to throw. A page that pre-seeds
  // `window.__rutterRefStore` with something else must not take the
  // snapshot down.
  const page = pageOver(el('body', {}, [el('button', {}, ['x'])]), {
    __rutterRefStore: { map: new WeakMap() },
  });
  const envelope = page.run(SERIALIZER_JS);
  assert.equal(envelope.truncated, true, 'a store that cannot mint refs is owned up to');
  assert.equal(envelope.root.children[0].ref, undefined, 'and no element is addressable');
});

test('a document with no body still reports a root', () => {
  const page = createPage({
    document: makeDocument({ noBody: true }).document,
    window: { innerWidth: 0, innerHeight: 0 },
  });
  const envelope = page.run(SERIALIZER_JS);
  assert.equal(envelope.version, 1);
  assert.equal(envelope.truncated, false);
  assert.equal(envelope.viewport.width, 0);
  assert.equal(envelope.root.role, 'generic');
  assert.equal([...envelope.root.children].length, 0);
});

test('an offscreen element is omitted rather than guessed at', () => {
  const page = pageOver(
    el('body', {}, [el('p', {}, ['shown']), el('p', { rects: false }, ['hidden'])])
  );
  const envelope = page.run(SERIALIZER_JS);
  assert.equal(
    envelope.root.children.length,
    1,
    'a layout-less element contributes nothing to the tree'
  );
});

test('a shadow host reports its shadow tree, not its light children', () => {
  const page = pageOver(
    el('body', {}, [
      el(
        'div',
        { shadow: el('span', {}, [el('a', { attrs: { href: '/shadow' } }, ['shadow child'])]) },
        [el('a', { attrs: { href: '/light' } }, ['light child'])]
      ),
    ])
  );
  const envelope = page.run(SERIALIZER_JS);
  const host = envelope.root.children[0];
  assert.equal(host.children.length, 1, 'the host contributes exactly one child');
  assert.equal(
    host.children[0].name,
    'shadow child',
    'the shadow tree is walked; an unrendered light child is omitted'
  );
});

test('a role with no text name stays unnamed', () => {
  // ARIA: a container must not inherit the page text as its name.
  const page = pageOver(
    el('body', {}, [el('div', { attrs: { role: 'group' } }, ['a lot of page text'])])
  );
  const envelope = page.run(SERIALIZER_JS);
  assert.equal(envelope.root.children[0].name, undefined);
});

test('a text name folds a growing window, not the whole subtree', () => {
  // The name may fold the subtree's textContent; a text-dense
  // container must not pay a whole-subtree fold on every snapshot for
  // a clip that keeps 120 characters. The windowed fold therefore has
  // to answer exactly what a fold of the full text answered, in both
  // directions: a name long enough to clip, and one the whitespace
  // delays past the first window. `fullFoldClip` restates the
  // full-text path as the reference.
  const fullFoldClip = (text, limit = 120) => {
    const folded = text.replace(/\s+/g, ' ').trim();
    if (folded.length <= limit) return folded;
    let sliced = folded.slice(0, limit - 3);
    const last = sliced.charCodeAt(sliced.length - 1);
    if (last >= 0xd800 && last <= 0xdbff) sliced = sliced.slice(0, -1);
    return `${sliced}...`;
  };

  const long = 'word '.repeat(100).trimEnd();
  const page = pageOver(el('body', {}, [el('li', {}, [long])]));
  const name = page.run(SERIALIZER_JS).root.children[0].name;
  assert.equal(
    name,
    fullFoldClip(long),
    'a 500-character name clips to what the full fold produced'
  );

  // Mostly whitespace: the first window folds to a fragment far below
  // the limit, so the window must keep growing until the clip matches
  // the full fold instead of clipping the fragment.
  const padded = `${' '.repeat(1000)}tail of a very ${'loud '.repeat(40)}name`;
  const paddedName = pageOver(
    el('body', {}, [el('li', {}, [padded])])
  ).run(SERIALIZER_JS).root.children[0].name;
  assert.equal(paddedName, fullFoldClip(padded), 'the window grew until the clip matched');

  // An all-whitespace name stays unnamed, as the full fold answered.
  const blank = pageOver(el('body', {}, [el('li', {}, [' '.repeat(2000)])]));
  assert.equal(blank.run(SERIALIZER_JS).root.children[0].name, undefined);
});
