/**
 * @fileoverview Behavioural suite for the document-start entropy capture.
 *
 * `entropy.js` exists so that a page cannot choose the ref scope its own
 * documents are given: the engine installs it before any page script
 * runs, and it binds the native `crypto.getRandomValues` onto the global
 * in a way a page cannot undo. These tests run the shipped asset and
 * then act like the page — replacing `crypto`, assigning over the
 * property, deleting it — and assert the captured generator survives
 * every one of those.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createPage, el, loadScript, makeDocument } from './dom.mjs';

const ENTROPY_JS = loadScript('crates/observe/src/assets/entropy.js');

/** A page whose `crypto` counts calls and fills with a known byte. */
function pageWithRecordingCrypto(byte) {
  const calls = [];
  const crypto = {
    getRandomValues(bytes) {
      calls.push(bytes.length);
      bytes.fill(byte);
      return bytes;
    },
  };
  const document = makeDocument({ body: el('body', {}, [el('button', {}, ['Save'])]) }).document;
  const page = createPage({ document, window: { crypto } });
  return { page, calls };
}

test('the capture binds the generator that was in place when it ran', () => {
  // The engine installs the script at document start, so the function it
  // binds is the native one. Everything the page does to `crypto`
  // afterwards must not change what the captured function returns.
  const { page, calls } = pageWithRecordingCrypto(7);
  page.run(ENTROPY_JS);

  page.window.crypto = {
    getRandomValues(bytes) {
      bytes.fill(0);
      return bytes;
    },
  };

  const bytes = new Uint8Array(4);
  page.window.__rutterGetRandomValues(bytes);
  assert.deepEqual([...bytes], [7, 7, 7, 7], 'the captured generator still fills');
  assert.deepEqual(calls, [4], 'and it is the one that was bound, not the replacement');
});

test('a page script cannot replace or delete the captured property', () => {
  const { page } = pageWithRecordingCrypto(7);
  page.run(ENTROPY_JS);

  const descriptor = Object.getOwnPropertyDescriptor(page.window, '__rutterGetRandomValues');
  assert.equal(descriptor.writable, false, 'the property is not writable');
  assert.equal(descriptor.configurable, false, 'the property is not configurable');
  assert.equal(descriptor.enumerable, false, 'the property is not enumerable');

  // A page script is sloppy by default, so both of these are silent
  // no-ops there rather than errors; what matters is the outcome.
  page.run('__rutterGetRandomValues = 1;');
  page.run('delete __rutterGetRandomValues;');
  assert.equal(
    typeof page.window.__rutterGetRandomValues,
    'function',
    'the captured generator is still there'
  );

  assert.throws(
    () => Object.defineProperty(page.window, '__rutterGetRandomValues', { value: 1 }),
    TypeError,
    'redefining it throws'
  );
});

test('a document without a usable crypto gets no capture and no throw', () => {
  // The script runs in the page, so it owes the same "never throws"
  // guarantee the serializer has: a document with no `crypto` leaves the
  // serializer to its fallback and degradation.
  const document = makeDocument({ body: el('body', {}, []) }).document;
  const page = createPage({ document, window: { crypto: undefined } });
  assert.doesNotThrow(() => page.run(ENTROPY_JS));
  assert.equal(page.window.__rutterGetRandomValues, undefined, 'nothing was captured');
});

test('a page whose crypto throws is captured, and the failure surfaces on use', () => {
  // Binding a function does not call it, so the capture still succeeds
  // and the property is locked like any other. The page's failure lands
  // where the serializer calls the generator, inside the guard that
  // degrades the store -- which is the path the serializer suite pins.
  const document = makeDocument({ body: el('body', {}, []) }).document;
  const page = createPage({
    document,
    window: {
      crypto: {
        getRandomValues() {
          throw new Error('no entropy for you');
        },
      },
    },
  });
  assert.doesNotThrow(() => page.run(ENTROPY_JS));
  assert.throws(
    () => page.window.__rutterGetRandomValues(new Uint8Array(4)),
    /no entropy for you/,
    'the page failure surfaces when the generator is used'
  );
});
