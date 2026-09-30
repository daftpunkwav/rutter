/**
 * @fileoverview Behavioural tests for the select-option page script.
 *
 * Boundary: runs the real SELECT_TEMPLATE shipped inside
 * `crates/observe/src/resolver.rs` and asserts what it does to the
 * element. The template lives in Rust source because the builder
 * splices the reference and the values into it; the test reads the
 * same source and splices the same way, so a rename fails loudly
 * here instead of silently testing a copy.
 *
 * The pinned behaviour: nothing matched means the selection is left
 * exactly as it was -- the Rust side refuses the action, and a
 * refused action must not clear the select as a side effect.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createPage, loadScript } from './dom.mjs';

const RESOLVER_SOURCE = loadScript('crates/observe/src/resolver.rs');

/** Splices the shipped template the way `select_script` does. */
function selectScript(reference, values) {
  const match = RESOLVER_SOURCE.match(/const SELECT_TEMPLATE: &str = r#"([\s\S]*?)"#;/);
  assert.ok(match, 'the shipped SELECT_TEMPLATE must be extractable from resolver.rs');
  return match[1].replace('__REF__', reference).replace('__VALUES__', JSON.stringify(values));
}

/** A select double carrying options shaped like real ones. */
function makeSelect(values, selectedAt) {
  return {
    tagName: 'SELECT',
    isConnected: true,
    options: values.map((value, index) => ({ value, selected: selectedAt.includes(index) })),
    dispatched: [],
    dispatchEvent(event) {
      this.dispatched.push(event.type);
    },
  };
}

/** A page sandbox whose ref store maps `e1` to `element`. */
function pageOver(element) {
  return createPage({
    window: { __rutterRefStore: { reverse: new Map([['e1', new WeakRef(element)]]) } },
    extra: { Event },
  });
}

test('matching values select exactly those options and fire the events', () => {
  const select = makeSelect(['a', 'b', 'c'], [0]);
  const answer = pageOver(select).run(selectScript('e1', ['b', 'c']));
  assert.deepEqual(
    select.options.map((option) => option.selected),
    [false, true, true],
    'the previously selected option is deselected with it'
  );
  assert.equal(answer.matched, 2);
  assert.deepEqual(select.dispatched, ['input', 'change']);
});

test('nothing matched leaves the selection and the events untouched', () => {
  const select = makeSelect(['a', 'b'], [1]);
  const answer = pageOver(select).run(selectScript('e1', ['zzz']));
  assert.equal(answer.matched, 0);
  assert.deepEqual(
    select.options.map((option) => option.selected),
    [false, true],
    'a refused action must not clear the select'
  );
  assert.deepEqual(select.dispatched, [], 'no change event may fire');
});

test('a missing reference is reported without touching anything', () => {
  const page = createPage({
    window: { __rutterRefStore: { reverse: new Map() } },
    extra: { Event },
  });
  const answer = page.run(selectScript('e1', ['a']));
  assert.equal(answer.missing, true);
});

test('an element that is not a select is reported as such', () => {
  const answer = pageOver({ tagName: 'DIV', isConnected: true }).run(
    selectScript('e1', ['a'])
  );
  assert.equal(answer.not_select, true);
});
