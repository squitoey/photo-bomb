// Dev-only smoke test. Runs only when the app is started with PHOTOBOMB_DEBUG_DIR set.
// It clicks through the main screens and writes a log plus HTML snapshots to that folder,
// so the UI can be checked without a screen. Waits are short on purpose: macOS suspends a
// web view a few seconds after launch when its window is not visible. It applies and then undoes one hard-link batch.
import { invoke } from "@tauri-apps/api/core";

export async function maybeSelfTest() {
  if (!(await invoke<boolean>("debug_enabled"))) return;
  const log: string[] = [];
  const say = (s: string) => {
    log.push(s);
    return invoke("debug_write", { name: "log.txt", content: log.join("\n") + "\n" });
  };
  window.addEventListener("error", (e) => say(`ERROR ${e.message}`));
  window.addEventListener("unhandledrejection", (e) => say(`REJECTION ${String(e.reason)}`));
  document.addEventListener("securitypolicyviolation", (e) => say(`CSP ${e.violatedDirective} ${e.blockedURI}`));

  // Sleep on the Rust side: timers in a hidden or napping web view are throttled.
  const wait = (ms: number) => invoke("debug_sleep", { ms });
  const click = async (text: string) => {
    const b = [...document.querySelectorAll("button")].find((el) => el.textContent?.trim().startsWith(text));
    if (!b) return say(`MISSING button "${text}"`);
    if (b.disabled) return say(`DISABLED button "${text}"`);
    b.click();
    await wait(100);
  };
  const snap = async (name: string) => {
    await wait(450);
    const imgs = [...document.images];
    const loaded = imgs.filter((i) => i.complete && i.naturalWidth > 0).length;
    await say(
      `${name}: ${imgs.length} images, ${loaded} loaded | ${document.querySelector(".summary")?.textContent ?? ""}`,
    );
    await invoke("debug_write", { name: `${name}.html`, content: document.documentElement.outerHTML });
  };

  try {
    await run();
  } catch (e) {
    await say(`SELFTEST FAILED ${String(e)} ${(e as Error)?.stack ?? ""}`);
  }

  async function run() {
    await wait(300);
    await snap("1-sources");
    await click("Exact duplicates");
    await snap("2-exact");
    await click("Auto-select all");
    await click("Review");
    await snap("3-review-trash");
    document.querySelectorAll<HTMLInputElement>(".choices input")[2]?.click();
    await snap("4-review-hardlink");
    await click("Replace with hard links");
    await snap("5-applied");
    await click("Undo");
    await snap("6-undone");
    await click("Close");
    await click("Similar photos");
    await snap("7-similar");
    await click("Compare");
    await snap("8-compare");
    await click("Info");
    await snap("8b-info");
    await click("Close");
    await click("Close");
    await click("⋯");
    await snap("8c-menu");
    await click("Not a duplicate");
    await snap("8d-not-a-duplicate");
    await click("Other photos");
    await snap("8e-others");
    await click("Sources");
    await snap("8f-sources");
    await click("History");
    await snap("9-history");
    await say("DONE");
  }
}
