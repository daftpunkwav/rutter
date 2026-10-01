// The browser's permission gate: web content's permission requests and
// Chromium's synchronous permission checks resolve through this one
// predicate. The policy is default-deny — an unlisted capability is
// refused — and only the three powers that cannot pull data off the
// machine are granted: fullscreen, pointerLock, and the sanitized
// clipboard write (a page may place text on the clipboard but can
// never read it back). Lives in its own module so tests/js can execute
// the shipped predicate against its permission list, the same way
// scheme-gate.js is tested.
const ALLOWED_PERMISSIONS = new Set([
  "fullscreen",
  "pointerLock",
  "clipboard-sanitized-write",
]);

function isPermissionGranted(permission) {
  return ALLOWED_PERMISSIONS.has(permission);
}

module.exports = { isPermissionGranted };
