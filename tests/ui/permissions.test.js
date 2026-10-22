// Every page as a caller with less than everything: no request on load may be
// refused (a refused read shows as an empty or wrong value), and every control
// the caller may not use is greyed out with the permission named, never live.
import { test, expect } from "@playwright/test";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
const RUN = Date.now().toString(36);
const PRODUCER = `gate-fixture-${RUN}`;
const OTHER = `gate-other-${RUN}`;
const PASSWORD = "Gate-Passw0rd-123";
const PERSONAS = {
  plain: `gate-plain-${RUN}`,
  usermanager: `gate-umgr-${RUN}`,
  maintainer: `gate-maint-${RUN}`,
};
const PAGES = [
  "/account.html",
  `/producers.html?service=${PRODUCER}`,
  `/producers.html?service=${PRODUCER}&api_type=openapi&version=1.0.0`,
  "/consumers.html",
  "/graph.html",
  "/reports.html",
  "/admin.html",
  "/observability.html",
];
const SPEC = (title) =>
  [
    "openapi: 3.0.0",
    "info:",
    `  title: ${title}`,
    "  version: 1.0.0",
    "paths:",
    "  /gate:",
    "    get:",
    "      responses:",
    "        '200':",
    "          description: OK",
  ].join("\n");

test.beforeAll(async ({ request }) => {
  const login = await request.post("/auth/login", {
    data: { username: "root", password: adminPassword },
  });
  const { token } = await login.json();
  const headers = { Authorization: `Bearer ${token}` };
  for (const producer of [PRODUCER, OTHER]) {
    const res = await request.post("/provide", {
      headers,
      data: { producername: producer, stability: "snapshot", openapi_yaml: SPEC(producer) },
    });
    expect(res.ok(), `provide ${producer}`).toBeTruthy();
  }
  // Registration starts closed; open it for the personas.
  const opened = await request.post("/admin/settings/registration", {
    headers,
    data: { enabled: true },
  });
  expect(opened.ok(), "open registration").toBeTruthy();
  for (const username of Object.values(PERSONAS)) {
    await request.post("/auth/register", { data: { username, password: PASSWORD } });
  }
  const users = await (await request.get("/admin/users", { headers })).json();
  const idOf = (name) => users.find((u) => u.username === name).id;
  for (const username of Object.values(PERSONAS)) {
    await request.post(`/admin/users/${idOf(username)}/approve`, { headers });
  }
  let res = await request.post(`/admin/users/${idOf(PERSONAS.usermanager)}/roles`, {
    headers,
    data: { role: "user_manager" },
  });
  expect(res.ok(), "grant user_manager").toBeTruthy();
  res = await request.post(`/admin/producers/${PRODUCER}/maintainers`, {
    headers,
    data: { user_id: idOf(PERSONAS.maintainer) },
  });
  expect(res.ok(), "assign maintainer").toBeTruthy();
});

async function signIn(page, username) {
  // A fresh start: the previous persona's session must not carry over.
  await page.context().clearCookies();
  await page.goto("/account.html");
  await page.evaluate(() => localStorage.clear());
  await page.goto("/account.html");
  await page.waitForSelector("#login-username", { state: "visible" });
  await page.fill('input[id="login-username"]', username);
  await page.fill('input[id="login-password"]', username === "root" ? adminPassword : PASSWORD);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
}

for (const [persona, username] of Object.entries(PERSONAS)) {
  test(`no page draws a refused request for the ${persona}`, async ({ page }) => {
    const refused = [];
    page.on("response", (res) => {
      if ([401, 403].includes(res.status())) {
        refused.push(`${res.status()} ${res.request().method()} ${new URL(res.url()).pathname}`);
      }
    });
    await signIn(page, username);
    for (const path of PAGES) {
      await page.goto(path);
      // Not "networkidle": the live-update stream stays open by design.
      await page.waitForLoadState("load");
      await page.waitForTimeout(1500);
    }
    expect(refused, `refused while loading as ${persona}`).toEqual([]);
  });
}

test("a gated control is greyed with the permission named, a granted one is live", async ({
  page,
}) => {
  const versionDelete = (producer) =>
    page.locator(".endpoint-card", { hasText: "1.0.0" }).locator(`[data-delete]`).first();

  await signIn(page, PERSONAS.plain);
  await page.goto(`/producers.html?service=${PRODUCER}`);
  const plainDelete = versionDelete(PRODUCER);
  await expect(plainDelete).toBeDisabled({ timeout: 10000 });
  await expect(plainDelete).toHaveAttribute("title", /manage_producers/);
  await expect(page.locator("[data-promote]").first()).toBeDisabled();

  // The maintainer may delete on their own Producer, not on another one.
  await signIn(page, PERSONAS.maintainer);
  await page.goto(`/producers.html?service=${PRODUCER}`);
  await expect(versionDelete(PRODUCER)).toBeEnabled({ timeout: 10000 });
  await page.goto(`/producers.html?service=${OTHER}`);
  await expect(versionDelete(OTHER)).toBeDisabled({ timeout: 10000 });

  // A user manager may not change the auto-approve switch (it sits in the
  // Local-users panel, shown to those who manage the auth configuration), and
  // sees the user list live.
  await signIn(page, PERSONAS.usermanager);
  await page.goto("/admin.html");
  await expect(page.locator("#admin-dashboard")).toBeVisible({ timeout: 10000 });
  const toggle = page.locator("#auto-approve-toggle");
  await expect(toggle).toBeDisabled();
  await expect(toggle).toHaveAttribute("title", /manage_settings/);
  await expect(page.locator("#snapshot-max-age-days")).toHaveValue(/\d+/);
  await expect(page.locator("#users-list")).toContainText(PERSONAS.plain, { timeout: 10000 });
  // A user manager also holds manage_roles; the auth configuration is not theirs.
  await expect(page.locator("#auth-config-locked [data-locked]")).toContainText(
    "manage_auth_config",
  );
  await expect(page.locator("#ldap-server-url")).toBeDisabled();
});

// The live updates use the stream that sends the Authorization header. The
// former WebSocket mode put the token into the URL, which the server refuses;
// its old switch in local storage must not bring it back.
test("live updates authenticate with the header, never a token in the URL", async ({ page }) => {
  const streams = [];
  const sockets = [];
  page.on("websocket", (ws) => sockets.push(ws.url()));
  page.on("request", (req) => {
    const url = new URL(req.url());
    if (url.pathname.startsWith("/api/sse/") || url.pathname.startsWith("/api/ws/")) {
      streams.push({ url, authorization: req.headers()["authorization"] });
    }
  });
  await signIn(page, PERSONAS.plain);
  await page.evaluate(() => localStorage.setItem("sanshain_use_ws", "true"));
  streams.length = 0;
  await page.goto("/");
  await expect.poll(() => streams.length + sockets.length).toBeGreaterThan(0);
  expect(sockets).toEqual([]);
  for (const { url, authorization } of streams) {
    expect(url.pathname).toBe("/api/sse/updates");
    expect(url.search).toBe("");
    expect(authorization).toMatch(/^Bearer \S+$/);
  }
});
