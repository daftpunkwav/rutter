/**
 * @fileoverview Behavioural tests for the browser's permission gate.
 *
 * Boundary: runs `browser/permission-gate.js` -- the shipped predicate
 * both permission entry points (async requests and Chromium's
 * synchronous checks) resolve through -- and asserts which powers web
 * content may use. The gate is security-relevant: a privacy-touching
 * capability that slips through (geolocation, camera, clipboard read)
 * lets a page pull data off the machine, so the refusals are pinned
 * next to the acceptances, including the case and shape edge cases a
 * looser lookup would let by.
 *
 * No build step and no test framework: `node --test` is a builtin.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { isPermissionGranted } = require('../../browser/permission-gate.js');

test('the three no-risk powers pass the gate', () => {
  for (const permission of [
    'fullscreen', // render full screen: no data leaves the machine
    'pointerLock', // capture the pointer: no data leaves the machine
    'clipboard-sanitized-write', // write text, never read it back
  ]) {
    assert.equal(
      isPermissionGranted(permission),
      true,
      `${permission} must be granted`
    );
  }
});

test('privacy-touching and device powers are refused', () => {
  for (const permission of [
    'geolocation', // the machine's location
    'media', // camera and microphone
    'notifications',
    'midi', // and midiSysex below: raw device access
    'midiSysex',
    'clipboard-read', // the read side the write grant must not imply
    'display-capture', // screen contents
    'speaker-selection',
    'openExternal', // launches handlers outside the browser
    'window-management',
    'mediaKeySystem', // DRM identification
    'fileSystem',
    'persistentStorage',
  ]) {
    assert.equal(
      isPermissionGranted(permission),
      false,
      `${permission} must be refused`
    );
  }
});

test('unknown names and odd shapes are refused', () => {
  for (const permission of [
    'Fullscreen', // Electron's names are matched exactly
    'FULLSCREEN',
    'fullscreen ', // no stray whitespace
    ' fullscreen',
    'fullscreen;media', // no composite shapes
    'fullscreen\u0000', // no truncated lookalikes
    'not-a-permission',
    '', // an empty name is nothing, not everything
    undefined,
    null,
    0,
  ]) {
    assert.equal(
      isPermissionGranted(permission),
      false,
      `${JSON.stringify(permission)} must be refused`
    );
  }
});
