/**
 * #269: what a script can check of a screen at phone width. Each check runs in the page and
 * returns one line per problem, naming the element, so a failure says where to look.
 *
 * - `sideways`: the document, or the app's scrolling panel (the `overflow: auto` page body inside
 *   `#main`), is wider than its box, so the page scrolls sideways.
 * - `smallTargets`: a link, button or form control smaller than 24 × 24 px whose 24 px circle,
 *   centred on it, overlaps another target (WCAG 2.2 2.5.8). Links inside a sentence are exempt,
 *   as 2.5.8 exempts them.
 * - `smallText`: visible text set under 11 px.
 * - `truncated`: an element that cuts its text off with an ellipsis, hiding more than a few
 *   characters, unless it is on the allow-list below (its full text is shown or reachable some
 *   other way). The hidden amount is estimated from the overflow width at half the font size per
 *   character.
 */
import type { Page } from "@playwright/test";

export type MobileProblems = { sideways: string[]; smallTargets: string[]; smallText: string[]; truncated: string[] };

/**
 * Ellipsis that is fine on a phone: each entry's full text is visible, or a tap away, elsewhere.
 * A selector matched with `Element.closest`, and why.
 */
export const TRUNCATION_ALLOWED: { selector: string; reason: string }[] = [];

/** Hidden characters at or under this are fine (a word's last letters). */
const HIDDEN_CHARS_ALLOWED = 3;
const MIN_TARGET = 24;
const MIN_TEXT_PX = 11;

/** Checks the page; `allowed` stands in for `TRUNCATION_ALLOWED` (the self-test passes its own). */
export async function findMobileProblems(page: Page, allowed: { selector: string; reason: string }[] = TRUNCATION_ALLOWED): Promise<MobileProblems> {
  return page.evaluate(
    ({ allowed, hiddenCharsAllowed, minTarget, minText }) => {
      const describe = (element: Element) => {
        const id = element.id ? `#${element.id}` : "";
        const label = (element.getAttribute("aria-label") ?? (element as HTMLElement).innerText ?? element.textContent ?? "").trim().replace(/\s+/g, " ").slice(0, 40);
        return `<${element.tagName.toLowerCase()}${id}>${label ? ` "${label}"` : ""}`;
      };
      const visible = (element: Element) => {
        const style = getComputedStyle(element);
        if (style.visibility === "hidden" || style.display === "none" || Number(style.opacity) === 0) return false;
        // Visually hidden until focused (sr-only: clipped), like the skip link.
        if (style.position === "absolute" && (style.clip.startsWith("rect(0") || /inset\(50%/.test(style.clipPath))) return false;
        const box = element.getBoundingClientRect();
        // Visually hidden (sr-only: 1 px, clipped) is for screen readers, not for the eye or a finger.
        return box.width > 1 && box.height > 1 && element.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true });
      };

      // 1. Sideways scrolling: the document, and the scrolling panels inside the main area.
      const sideways: string[] = [];
      const root = document.documentElement;
      if (root.scrollWidth > root.clientWidth + 1) sideways.push(`the document is ${root.scrollWidth} px wide in a ${root.clientWidth} px viewport`);
      // The page's panel is the outermost scroll container in `#main`; a wide table's own
      // scroller inside it may scroll sideways, as the epic allows.
      const scrolls = (element: Element) => /(auto|scroll)/.test(getComputedStyle(element).overflowY) && element.clientHeight >= 200;
      const panels = [...document.querySelectorAll("#main *")].filter((element) => {
        if (!scrolls(element)) return false;
        for (let parent = element.parentElement; parent && parent.id !== "main"; parent = parent.parentElement) if (scrolls(parent)) return false;
        return true;
      });
      for (const panel of panels) {
        if (panel.scrollWidth > panel.clientWidth + 1) {
          const widest = [...panel.querySelectorAll("*")]
            .filter((child) => child.getBoundingClientRect().right > panel.getBoundingClientRect().right + 1)
            .sort((a, b) => b.getBoundingClientRect().right - a.getBoundingClientRect().right)[0];
          sideways.push(`the scrolling panel is ${panel.scrollWidth} px wide in ${panel.clientWidth} px${widest ? `; widest: ${describe(widest)}` : ""}`);
        }
      }

      // 2. Targets: under 24 × 24 px, and its 24 px circle overlaps another target.
      const targetSelector = "a[href], button, input:not([type=hidden]), select, textarea, summary, [role=button], [role=link], [role=tab], [role=checkbox], [role=switch], [role=radio], [role=menuitem], [role=option]";
      const targets = [...document.querySelectorAll(targetSelector)].filter((element) => visible(element) && !element.closest("[inert], [aria-hidden=true]"));
      // A target inside another target (an icon button's inner span with a role) counts once.
      const outer = targets.filter((element) => !targets.some((other) => other !== element && other.contains(element)));
      const inSentence = (element: Element) => {
        if (element.tagName !== "A" || getComputedStyle(element).display !== "inline") return false;
        const parent = element.parentElement;
        return Boolean(parent && [...parent.childNodes].some((node) => node.nodeType === Node.TEXT_NODE && (node.textContent ?? "").trim().length > 0));
      };
      const boxes = outer.map((element) => ({ element, box: element.getBoundingClientRect() }));
      const smallTargets: string[] = [];
      const undersized = (b: DOMRect) => b.width < minTarget - 0.5 || b.height < minTarget - 0.5;
      const radius = minTarget / 2;
      for (const { element, box } of boxes) {
        if (!undersized(box)) continue;
        if (inSentence(element)) continue;
        const cx = box.left + box.width / 2;
        const cy = box.top + box.height / 2;
        const overlapped = boxes.find(({ element: other, box: b }) => {
          if (other === element) return false;
          // The circle overlaps the other target when the box's nearest point is within the radius.
          const nx = Math.max(b.left, Math.min(cx, b.right));
          const ny = Math.max(b.top, Math.min(cy, b.bottom));
          if (Math.hypot(nx - cx, ny - cy) < radius) return true;
          // Or another undersized target's circle: two 24 px circles overlap when their centres
          // are under 24 px apart (2.5.8's spacing).
          if (!undersized(b) || inSentence(other)) return false;
          return Math.hypot(b.left + b.width / 2 - cx, b.top + b.height / 2 - cy) < minTarget;
        });
        if (overlapped) smallTargets.push(`${describe(element)} is ${Math.round(box.width)} × ${Math.round(box.height)} px, too close to ${describe(overlapped.element)}`);
      }

      // 3. Text under 11 px: each visible text node's own element.
      const smallText = new Set<string>();
      const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        const text = (node.textContent ?? "").trim();
        const parent = node.parentElement;
        if (!text || !parent || !visible(parent)) continue;
        const size = parseFloat(getComputedStyle(parent).fontSize);
        // One line per place: a row of small key caps is one problem, not one per key.
        if (size < minText - 0.01) smallText.add(`${size} px in ${describe(parent.closest("button, a, td, th, li, p, h1, h2, h3, label, div") ?? parent)}`);
      }

      // 4. Ellipsis that hides more than a few characters, with no allow-list entry.
      const truncated: string[] = [];
      for (const element of document.querySelectorAll("body *")) {
        const style = getComputedStyle(element);
        if (style.textOverflow !== "ellipsis" || !visible(element)) continue;
        const hidden = element.scrollWidth - element.clientWidth;
        if (hidden <= 1) continue;
        const chars = Math.round(hidden / (parseFloat(style.fontSize) * 0.5));
        if (chars <= hiddenCharsAllowed) continue;
        if (allowed.some((entry) => element.closest(entry))) continue;
        truncated.push(`${describe(element)} hides about ${chars} characters`);
      }

      return { sideways, smallTargets, smallText: [...smallText], truncated };
    },
    { allowed: allowed.map((entry) => entry.selector), hiddenCharsAllowed: HIDDEN_CHARS_ALLOWED, minTarget: MIN_TARGET, minText: MIN_TEXT_PX },
  );
}

/**
 * Saves screenshots of the screen into `dir`, one per panel's height: the app scrolls inside a
 * panel, so a full-page screenshot would show only the first screenful.
 */
export async function saveScreens(page: Page, dir: string, name: string) {
  const panel = page.locator("#main [data-page-body], #main .overflow-auto").first();
  const scrollable = (await panel.count()) > 0;
  const [height, total] = scrollable ? await panel.evaluate((element) => [element.clientHeight, element.scrollHeight]) : [1, 1];
  const segments = Math.min(8, Math.max(1, Math.ceil(total / Math.max(1, height - 40))));
  for (let index = 0; index < segments; index += 1) {
    if (scrollable) await panel.evaluate((element, top) => element.scrollTo({ top }), index * (height - 40));
    await page.screenshot({ path: `${dir}/${name}-${index + 1}.png` });
  }
  if (scrollable) await panel.evaluate((element) => element.scrollTo({ top: 0 }));
}
