// The shared view's scheme gate: only web pages (and the blank start
// target) may drive the shared view. Both entry points that can
// navigate it — web content's window.open and the toolbar's navigate
// channel — test this one predicate, so the whitelist exists once and
// the two paths cannot drift into disagreeing about what may reach the
// page (no file:, devtools:, or javascript: URLs through the main
// process). Lives in its own module so tests/js can execute the
// shipped predicate against its scheme list.
function isWebSchemeUrl(url) {
  return /^https?:\/\//i.test(url) || url === "about:blank";
}

module.exports = { isWebSchemeUrl };
