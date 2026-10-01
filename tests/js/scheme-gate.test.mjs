/**
 * @fileoverview Behavioural tests for the browser's scheme gate.
 *
 * Boundary: runs `browser/scheme-gate.js` -- the shipped predicate both
 * navigation entry points (window.open and the toolbar's navigate
 * channel) test -- and asserts which URLs may reach the shared view.
 * The gate is security-relevant: a privileged scheme that slips through
 * lets web content or a hijacked toolbar drive the main process's view
 * to local files or script URLs, so the refusals are pinned next to the
 * acceptances, including the case shapes a looser regex would let by.
 *
 * No build step and no test framework: `node --test` is a builtin.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { isWebSchemeUrl } = require('../../browser/scheme-gate.js');

test('web pages and the blank start target pass the gate', () => {
  for (const url of [
    'http://example.com',
    'https://example.com/a/page?q=1',
    'HTTPS://EXAMPLE.COM/UPPER', // the scheme is case-insensitive
    'about:blank', // the start page the view loads at boot
  ]) {
    assert.equal(isWebSchemeUrl(url), true, `${url} must drive the shared view`);
  }
});

test('privileged and non-web schemes are refused', () => {
  for (const url of [
    'file:///etc/passwd',
    'file://host/share/https://evil.example', // a nested lookalike: no unanchored match
    'javascript:alert(1)',
    'JAVASCRIPT:alert(1)', // the refusal is case-insensitive too
    'devtools://devtools/bundled/inspector.html',
    'chrome://settings',
    'data:text/html,<script>alert(1)</script>',
    'about:config', // only blank about: is the start target
    'about:blank/extra', // not the exact start target
    'https:example.com', // scheme-relative lookalike without the slashes
    '//example.com', // protocol-relative
    '',
  ]) {
    assert.equal(isWebSchemeUrl(url), false, `${JSON.stringify(url)} must not drive the shared view`);
  }
});
