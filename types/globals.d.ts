// Globals of the page scripts that no script declares at its top level: the properties the
// scripts put on `window`, and the hooks a page may define. The top-level functions and
// variables of the scripts themselves need no declaration here: the scripts are classic
// scripts in one TypeScript program, so the compiler sees them as the globals they are.
// This file has no import or export, which keeps its declarations global.

interface Window {
  /** Set while the live-update stream is open (`static/js/common.js`). */
  _sseInitialized?: boolean;
  /** Switches between the light and the dark theme (`static/js/common.js`). */
  sanshainToggleTheme: () => void;
  /** Shows the page loader (`static/js/common.js`). */
  showLoader: () => void;
  /** Hides the page loader (`static/js/common.js`). */
  hideLoader: () => void;
  /** Set by the screenshot tests to skip the loader's fade and the wait for a sign-in. */
  SANSHAIN_FAST_SCREENSHOT?: boolean;
  /** A page may define this to refresh its lists after a favourite changed (`discovery.js`). */
  loadData?: () => unknown;
}

// The two loader functions are put on `window` and called by their bare names.
declare function showLoader(): void;
declare function hideLoader(): void;

/** A page defines this to react when the session ended (`apiCall` in `common.js`). */
declare function onSessionExpired(): void;

/** The options of `apiCall`: those of `fetch`, with a body that may be a plain object. */
interface ApiCallOptions extends Omit<RequestInit, "body" | "headers"> {
  headers?: Record<string, string>;
  /** An object is sent as JSON. */
  body?: BodyInit | object | null;
}

// JSON read from a response or a string is `unknown`, not `any` as the standard library has
// it: the reader says which shape it expects with a JSDoc cast, and nothing untyped flows on.
// A declaration merged in later wins over the library's own overload.
interface Body {
  json(): Promise<unknown>;
}
interface JSON {
  parse(text: string, reviver?: (this: unknown, key: string, value: unknown) => unknown): unknown;
}
