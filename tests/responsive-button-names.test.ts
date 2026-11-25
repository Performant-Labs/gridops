/**
 * #256: a button or link whose text is hidden below a breakpoint (`<span className="hidden
 * sm:inline">Refresh</span>` beside an icon) has no accessible name on a phone, where only the icon
 * shows (axe `button-name`, critical). #204's walkthrough found two, the fleet health page's Refresh
 * and the Overview's Refresh GitHub. This reads every component in `src` and requires such a control
 * to carry its name in `aria-label` (or `title`, which `AsyncActionButton` turns into one).
 */
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";

import ts from "typescript";
import { describe, expect, it } from "vitest";

const ROOT = new URL("..", import.meta.url).pathname;
const CONTROLS = new Set(["Button", "button", "a", "Link", "AsyncActionButton"]);
/** Hidden at the base width and shown from a breakpoint up: `hidden sm:inline`, `hidden md:inline-flex`. */
const RESPONSIVE_HIDDEN = /(?:^|\s)hidden(?:\s.*)?\s(?:sm|md|lg|xl):(?:inline|inline-flex|block|flex)(?:\s|$)/;

function components(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return components(path);
    return entry.name.endsWith(".tsx") ? [path] : [];
  });
}

const tagName = (node: ts.JsxOpeningLikeElement) => node.tagName.getText();
const attribute = (node: ts.JsxOpeningLikeElement, name: string) =>
  node.attributes.properties.find((property): property is ts.JsxAttribute => ts.isJsxAttribute(property) && property.name.getText() === name);
const literalClass = (node: ts.JsxOpeningLikeElement) => {
  const value = attribute(node, "className")?.initializer;
  return value && ts.isStringLiteral(value) ? value.text : "";
};

/** Whether `children` show text at every width: a text child, or an expression that is not an icon. */
function visibleText(children: ts.NodeArray<ts.JsxChild>): boolean {
  return children.some((child) => {
    if (ts.isJsxText(child)) return child.text.trim() !== "";
    if (ts.isJsxExpression(child)) return child.expression !== undefined && !ts.isJsxElement(child.expression) && !ts.isJsxSelfClosingElement(child.expression) && !ts.isConditionalExpression(child.expression);
    if (ts.isJsxElement(child)) return !RESPONSIVE_HIDDEN.test(literalClass(child.openingElement)) && visibleText(child.children);
    return false;
  });
}

const hidesText = (children: ts.NodeArray<ts.JsxChild>): boolean =>
  children.some((child) => ts.isJsxElement(child) && (RESPONSIVE_HIDDEN.test(literalClass(child.openingElement)) || hidesText(child.children)));

/** Controls in `file` that are icon-only below a breakpoint and carry no `aria-label` or `title`. */
function unnamed(file: string): string[] {
  const source = ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const found: string[] = [];
  const visit = (node: ts.Node) => {
    if (ts.isJsxElement(node) && CONTROLS.has(tagName(node.openingElement)) && hidesText(node.children) && !visibleText(node.children)) {
      const opening = node.openingElement;
      if (!attribute(opening, "aria-label") && !attribute(opening, "aria-labelledby") && !attribute(opening, "title")) {
        const { line } = source.getLineAndCharacterOfPosition(opening.getStart());
        found.push(`${relative(ROOT, file)}:${line + 1} <${tagName(opening)}>`);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return found;
}

describe("controls whose text is hidden on a phone", () => {
  it("finds the pattern (the check reads what it should)", () => {
    const sample = join(ROOT, "src/features/fleet-health/page.tsx");
    const source = readFileSync(sample, "utf8");
    expect(source).toMatch(/className="hidden sm:inline">Refresh</);
  });

  it("each carry an aria-label or title", () => {
    expect(components(join(ROOT, "src")).flatMap(unnamed)).toEqual([]);
  });
});
