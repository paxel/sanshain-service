// The stepper of the recorded walkthroughs (issue #57), against the built
// documentation site: step navigation by buttons, chips and arrow keys, the
// URL hash, the story graph's visibility and highlight classes, and the
// inlined evidence. Builds the site and serves it itself, so it needs no
// running service; part of `npm run test:ui`.
import { test, expect } from "@playwright/test";
import { spawn } from "node:child_process";
import net from "node:net";

let server;
let base;

function freePort() {
  return new Promise((resolve) => {
    const s = net.createServer();
    s.listen(0, "127.0.0.1", () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });
}

test.describe("Documentation site: the walkthrough stepper", () => {
  test.beforeAll(async () => {
    const port = await freePort();
    base = `http://127.0.0.1:${port}`;
    server = spawn(process.execPath, ["scripts/build-site.js", "--serve", String(port)], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    let output = "";
    await new Promise((resolve, reject) => {
      server.stdout.on("data", (chunk) => {
        output += chunk;
        if (output.includes("Serving ")) resolve();
      });
      server.stderr.on("data", (chunk) => (output += chunk));
      server.on("exit", (code) => reject(new Error(`build-site exited with ${code}:\n${output}`)));
    });
  });

  test.afterAll(() => {
    if (server) server.kill();
  });

  const visibleStep = (page) => page.locator(".story .step:not([hidden])");

  test("opens on step 1, hides the others, and lists every step", async ({ page }) => {
    await page.goto(`${base}/developer/story.html`);
    await expect(page.locator(".story.is-enhanced")).toHaveCount(1);
    await expect(visibleStep(page)).toHaveCount(1);
    await expect(visibleStep(page)).toHaveId("step-1");
    await expect(page.locator(".step-list a")).toHaveCount(12);
    await expect(page.locator(".step-list a[aria-current='step']")).toHaveAttribute(
      "href",
      "#step-1",
    );
    await expect(page.locator(".step-counter")).toHaveText("Step 1 of 12");
    await expect(page.locator(".step-prev")).toBeDisabled();
    await expect(page.locator(".step-next")).toBeEnabled();
  });

  test("next, previous, chips and arrow keys move between steps and set the hash", async ({
    page,
  }) => {
    await page.goto(`${base}/developer/story.html`);
    await page.click(".step-next");
    await expect(visibleStep(page)).toHaveId("step-2");
    expect(new URL(page.url()).hash).toBe("#step-2");
    await page.keyboard.press("ArrowRight");
    await expect(visibleStep(page)).toHaveId("step-3");
    await page.keyboard.press("ArrowLeft");
    await expect(visibleStep(page)).toHaveId("step-2");
    await page.click(".step-prev");
    await expect(visibleStep(page)).toHaveId("step-1");
    await page.click(".step-list a[href='#step-9']");
    await expect(visibleStep(page)).toHaveId("step-9");
    expect(new URL(page.url()).hash).toBe("#step-9");
    await expect(page.locator(".step-counter")).toHaveText("Step 9 of 12");
    await page.click(".step-list a[href='#step-12']");
    await expect(page.locator(".step-next")).toBeDisabled();
    // Navigation pushed history entries: Back returns to the previous step.
    await page.goBack();
    await expect(visibleStep(page)).toHaveId("step-9");
    expect(new URL(page.url()).hash).toBe("#step-9");
  });

  test("a #step-N hash opens that step; a hash inside a step opens the step too", async ({
    page,
  }) => {
    await page.goto(`${base}/operator/story.html#step-6`);
    await expect(visibleStep(page)).toHaveId("step-6");
    await expect(page.locator(".step-counter")).toHaveText("Step 6 of 10");
    const innerId = await page.locator("#step-8 h2").getAttribute("id");
    await page.goto(`${base}/operator/story.html#${innerId}`);
    await expect(visibleStep(page)).toHaveId("step-8");
    await page.evaluate(() => {
      window.location.hash = "#step-3";
    });
    await expect(visibleStep(page)).toHaveId("step-3");
  });

  test("the story graph shows and highlights what each step declares", async ({ page }) => {
    await page.goto(`${base}/developer/story.html`);
    const part = (name) => page.locator(`.story-graph [data-g='${name}']`);
    // Step 1: payments and its snapshot exist and are highlighted; orders does not exist yet.
    await expect(part("payments")).toHaveClass(/is-highlight/);
    await expect(part("payments")).not.toHaveClass(/is-hidden/);
    await expect(part("orders")).toHaveClass(/is-hidden/);
    await expect(part("pin")).toHaveClass(/is-hidden/);
    // Step 3: orders and the pin appear and are the highlight; payments stays, no longer highlighted.
    await page.click(".step-list a[href='#step-3']");
    await expect(part("orders")).not.toHaveClass(/is-hidden/);
    await expect(part("orders")).toHaveClass(/is-highlight/);
    await expect(part("pin")).toHaveClass(/is-highlight/);
    await expect(part("payments")).not.toHaveClass(/is-highlight/);
    await expect(part("branch")).toHaveClass(/is-hidden/);
    // Step 9: the branch appears.
    await page.click(".step-list a[href='#step-9']");
    await expect(part("branch")).not.toHaveClass(/is-hidden/);
    await expect(part("branch")).toHaveClass(/is-highlight/);
    await expect(page.locator(".story-graph figcaption")).toContainText("release-2026.10");
  });

  test("the recorded evidence is inlined into the steps", async ({ page }) => {
    await page.goto(`${base}/developer/story.html#step-4`);
    const evidence = page.locator("#step-4 pre[data-evidence='developer/04-forgotten-bump.http']");
    await expect(evidence).toContainText("HTTP/1.1 409 Conflict");
    await expect(evidence).toContainText('"proposed_version": "1.1.0"');
    await expect(page.locator("#step-3 img[src$='03-graph.png']")).toHaveCount(1);
    await page.goto(`${base}/operator/recipes.html`);
    await expect(
      page.locator("pre[data-evidence='operator/06-delivery-received.http']"),
    ).toContainText("Sanshain-Signature: t=");
  });
});
