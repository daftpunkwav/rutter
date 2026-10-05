/**
 * @fileoverview Behavioural suite for the document-start ref-scope minter.
 *
 * `entropy.js` exists so that a page cannot choose the ref scope its own
 * documents are given: the engine installs it before any page script
 * runs, and it captures the generator, the constructor, and the digit
 * table it needs while the platform is still the platform. These tests
 * run the shipped asset and then act like the page — replacing `crypto`,
 * replacing the prototypes a conversion would go through, assigning over
 * the property, deleting it — and assert the minter survives every one
 * of those.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createPage, el, loadScript, makeDocument } from './dom.mjs';

const ENTROPY_JS = loadScript('crates/observe/src/assets/entropy.js');

/** A page over a button, with a `crypto` that counts calls. */
function pageWithRecordingCrypto() {
  const calls = [];
  const crypto = {
    getRandomValues(bytes) {
      calls.push(bytes.length);
      bytes.fill(7);
      return bytes;
    },
  };
  const document = makeDocument({ body: el('body', {}, [el('button', {}, ['Save'])]) }).document;
  const page = createPage({ document, window: { crypto } });
  return { page, calls };
}

test('the minter draws from the generator that was in place when it ran', () => {
  // The engine installs the script at document start, so the function it
  // binds is the native one. Everything the page does to `crypto`
  // afterwards must not change what the minter returns.
  const { page, calls } = pageWithRecordingCrypto();
  page.run(ENTROPY_JS);

  page.window.crypto = {
    getRandomValues(bytes) {
      bytes.fill(0);
      return bytes;
    },
  };

  const scope = page.window.__rutterRefScope();
  assert.deepEqual(calls, [4], 'the bound generator is the one that was captured');
  assert.notEqual(scope, '0000', 'the replacement did not reach the minter');
  assert.match(scope, /^[0-9a-z]{4}$/, 'the documented scope shape');
});

test('a page cannot force a scope through the prototypes a conversion uses', () => {
  // The scope is converted to base36 digits, and a page can replace the
  // methods that conversion would normally go through. Everything the
  // minter needs was captured before the page ran, so patching
  // `Number.prototype.toString`, `String.prototype.charAt`, or the
  // typed-array iterator changes nothing.
  const { page } = pageWithRecordingCrypto();
  page.run(ENTROPY_JS);

  // Patched from inside the page, which is where a page's own script
  // runs: the minter captured all three before it.
  page.run("Number.prototype.toString = function () { return '0000'; };");
  page.run("String.prototype.charAt = function () { return '0'; };");
  page.run("String.prototype.padStart = function () { return '0000'; };");
  page.run('Uint8Array.prototype[Symbol.iterator] = function* () { yield 0; };');

  const scope = page.window.__rutterRefScope();
  assert.match(scope, /^[0-9a-z]{4}$/, 'the scope keeps its shape');
  assert.notEqual(scope, '0000', 'the patched conversion did not reach it');
  assert.equal(
    page.window.__rutterRefScope(),
    scope,
    'the captured generator still fills, so the same bytes give the same scope'
  );
});

test('a page script cannot replace or delete the minter', () => {
  const { page } = pageWithRecordingCrypto();
  page.run(ENTROPY_JS);

  const descriptor = Object.getOwnPropertyDescriptor(page.window, '__rutterRefScope');
  assert.equal(descriptor.writable, false, 'the property is not writable');
  assert.equal(descriptor.configurable, false, 'the property is not configurable');
  assert.equal(descriptor.enumerable, false, 'the property is not enumerable');

  // A page script is sloppy by default, so both of these are silent
  // no-ops there rather than errors; what matters is the outcome.
  page.run('__rutterRefScope = 1;');
  page.run('delete __rutterRefScope;');
  assert.equal(
    typeof page.window.__rutterRefScope,
    'function',
    'the minter is still there'
  );

  assert.throws(
    () => Object.defineProperty(page.window, '__rutterRefScope', { value: 1 }),
    TypeError,
    'redefining it throws'
  );
});

test('a document without a usable crypto gets no minter and no throw', () => {
  // The script runs in the page, so it owes the same "never throws"
  // guarantee the serializer has: a document with no `crypto` gets no
  // minter, and the serializer's store degrades rather than minting from
  // a source the page could replace.
  const document = makeDocument({ body: el('body', {}, []) }).document;
  const page = createPage({ document, window: { crypto: undefined } });
  assert.doesNotThrow(() => page.run(ENTROPY_JS));
  assert.equal(page.window.__rutterRefScope, undefined, 'nothing was installed');
});

test('a page whose crypto throws is captured, and the failure surfaces on use', () => {
  // Binding a function does not call it, so the capture still succeeds
  // and the property is locked like any other. The page's failure lands
  // where the serializer calls the minter, inside the guard that
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
    () => page.window.__rutterRefScope(),
    /no entropy for you/,
    'the page failure surfaces when the minter is used'
  );
});
