/** @type {import('tailwindcss').Config} */
// Replaces the runtime play-CDN (ai/improvements.md #11): the CDN scanned the
// live DOM, so a build-time scan must cover every source that emits a class —
// the HTML and every JS file that builds class strings. All dynamic classes in
// this codebase are whole-string ternaries, so their literal branches are seen.
module.exports = {
  content: ["./static/**/*.html", "./static/**/*.js"],
  theme: { extend: {} },
  plugins: [],
};
