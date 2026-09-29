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
  assert.match(button.ref, /^e\d+$/, 'an actionable element is addressable');
  assert.equal(generic.role, 'generic');
  assert.equal(generic.ref, undefined, 'a generic element is not actionable');
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
  assert.equal(store.counter, 601, 'a sweep renumbers nothing; the new ref is e601');
});

test('the sweep spares the refs whose elements are still alive', () => {
  // Dropping a live element's entry would break the stability the
  // snapshot format promises: the next snapshot must hand the same
  // element the same ref.
  const buttons = Array.from({ length: 600 }, (_, i) => el('button', {}, [`b${i}`]));
  const page = pageOver(el('body', {}, buttons));
  page.run(SERIALIZER_JS);
  const store = page.window.__rutterRefStore;
  assert.equal(collectRange(page, 0, 500).marked, 500, 'the first five hundred went away');

  const envelope = page.run(SERIALIZER_JS);
  assert.equal(store.reverse.size, 100, 'the 500 collected entries are gone, the live 100 stay');
  const rendered = [...envelope.root.children];
  assert.equal(rendered.length, 600, 'the same document is still fully rendered');
  assert.equal(
    rendered[500].ref,
    'e501',
    'a live element keeps the ref it was first given'
  );
  assert.equal(rendered[599].ref, 'e600', 'and so does the last');
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
