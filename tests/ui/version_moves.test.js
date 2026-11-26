// san-1031: the Producer page summarises what each version changed, the spec
// viewer folds each version open to the same list, and the Consumer page
// shows per Pin the newer versions on its own endpoints and the free move.
import { test, expect, json } from "./fixtures.js";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

/** @param {string} version @param {string} [text] @param {boolean} [orders] */
function spec(version, text = "OK", orders = true) {
  const lines = [
    "openapi: 3.0.3",
    "info:",
    "  title: Moves Fixture",
    `  version: ${version}`,
    "paths:",
    "  /users:",
    "    get:",
    "      responses:",
    "        '200':",
    `          description: ${text}`,
  ];
  if (orders) {
    lines.push("  /orders:", "    get:", "      responses:", "        '200':");
    lines.push(`          description: ${text}`);
  }
  return lines.join("\n") + "\n";
}

/** @param {import("@playwright/test").Page} page */
async function signIn(page) {
  await page.goto("/account.html");
  await page.waitForSelector("#login-username", { state: "visible" });
  await page.fill("#login-username", "root");
  await page.fill("#login-password", /** @type {string} */ (adminPassword));
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
}

test("a version shows its endpoint changes and a Consumer its free move", async ({
  page,
  request,
}) => {
  const stamp = Date.now().toString(36);
  const producer = `moves-api-${stamp}`;
  const consumer = `moves-app-${stamp}`;
  const login = await request.post("/auth/login", {
    data: { username: "root", password: adminPassword },
  });
  const { token } = /** @type {{ token: string }} */ (await json(login));
  const headers = { Authorization: `Bearer ${token}` };
  for (const openapi_yaml of [spec("1.0.0"), spec("1.1.0", "Fine"), spec("2.0.0", "OK", false)]) {
    const res = await request.post("/provide", {
      headers,
      data: { producername: producer, stability: "ga", openapi_yaml },
    });
    expect(res.ok(), await res.text()).toBeTruthy();
  }
  const required = await request.get("/require", {
    headers,
    params: {
      consumername: consumer,
      producername: producer,
      version: "1.0.0",
      path: "/orders",
      method: "GET",
    },
  });
  expect(required.ok()).toBeTruthy();
  expect(required.headers()["x-sanshain-upgrade"]).toBe("1.1.0");

  await signIn(page);
  await page.goto(`/producers.html?service=${producer}`);
  const card = page.locator('[data-endpoint-changes="openapi"]');
  await expect(card).toContainText("What each version changed", { timeout: 10000 });
  const v2 = card.locator('[data-endpoint-changes-version="2.0.0"]');
  await expect(v2).toContainText("−1 removed");
  await v2.locator("summary").click();
  await expect(v2.locator('[data-endpoint-change="removed"]')).toContainText("GET /orders");

  await page.goto(`/yaml.html?service=${producer}&api_type=openapi&version=2.0.0`);
  const fold = page.locator('.version-card[data-version="1.0.0"] [data-version-changes]');
  await expect(fold).toContainText("+2 added", { timeout: 10000 });

  await page.goto(`/consumers.html?name=${consumer}`);
  await expect(page.locator('[data-move-proposal="1.1.0"]')).toContainText(
    "nothing changes for the endpoints you require",
    { timeout: 10000 },
  );
  const moves = page.locator("[data-pin-moves]");
  await moves.locator("summary").click();
  await expect(moves.locator('[data-move-version="2.0.0"]')).toContainText("removed");
  await expect(moves.locator('[data-move-version="1.1.0"]')).toContainText("free move");
});

// san-1032: the strips on top of the Producer and Consumer pages.
test("the strips show who pins which version and what each require set holds", async ({
  page,
  request,
}) => {
  const stamp = Date.now().toString(36);
  const producer = `strip-api-${stamp}`;
  const consumer = `strip-app-${stamp}`;
  const login = await request.post("/auth/login", {
    data: { username: "root", password: adminPassword },
  });
  const { token } = /** @type {{ token: string }} */ (await json(login));
  const headers = { Authorization: `Bearer ${token}` };
  for (const openapi_yaml of [spec("1.0.0"), spec("1.1.0")]) {
    const res = await request.post("/provide", {
      headers,
      data: { producername: producer, stability: "ga", openapi_yaml, trunk: true },
    });
    expect(res.ok(), await res.text()).toBeTruthy();
  }
  for (const [version, hash] of [
    ["1.0.0", "aaa"],
    ["1.1.0", "bbb"],
  ]) {
    const res = await request.get("/require", {
      headers,
      params: {
        consumername: consumer,
        producername: producer,
        version: /** @type {string} */ (version),
        path: "/users",
        method: "GET",
        trunk: "true",
        requires_hash: /** @type {string} */ (hash),
      },
    });
    expect(res.ok(), await res.text()).toBeTruthy();
  }

  await signIn(page);
  await page.goto(`/consumers.html?name=${consumer}`);
  const strip = page.locator('[data-version-strip="consumer"]');
  await expect(strip.locator('[data-require-set="1"]')).toContainText(`${producer} 1.0.0`, {
    timeout: 10000,
  });
  await expect(strip.locator('[data-require-set="2"]')).toContainText(`${producer} 1.1.0`);

  await page.goto(`/producers.html?service=${producer}`);
  const bars = page.locator('[data-version-strip="producer"]');
  await expect(bars).toHaveAttribute("data-strip-bars", "2", { timeout: 10000 });
  await expect(bars.locator("svg").first()).toBeVisible();
  await bars.locator("[data-strip-scope]").selectOption("dev");
  await expect(bars).toHaveAttribute("data-strip-bars", "2");
});
