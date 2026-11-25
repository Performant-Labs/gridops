/**
 * #256: a button or link whose text is hidden below a breakpoint (`<span className="hidden
 * sm:inline">Refresh</span>` beside an icon) has no accessible name on a phone, where only the icon
 * shows (axe `button-name`, critical). #204's walkthrough found two, the fleet health page's Refresh
 * and the Overview's Refresh GitHub. This reads every component in `src` and requires such a control
 * to carry its name in `aria-label` (or `title`, which `AsyncActionButton` turns into one). A class
 * counts wherever it is written in `className`: a plain string, a template, or a string inside
 * `cn(...)`; hidden means `hidden <bp>:inline` (and the like) or `max-<bp>:hidden`.
 */
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";

import ts from "typescript";
import { describe, expect, it } from "vitest";

const ROOT = new URL("..", import.meta.url).pathname;
const CONTROLS = new Set(["Button", "button", "a", "Link", "AsyncActionButton"]);
/**
 * Hidden on a phone and shown from a breakpoint up: `hidden sm:inline`, `hidden md:inline-flex`, or
 * hidden only below one, `max-sm:hidden`.
 */
const RESPONSIVE_HIDDEN = [
  /(?:^|\s)hidden(?:\s.*)?\s(?:sm|md|lg|xl|2xl):(?:inline|inline-flex|inline-block|block|flex|grid|contents)(?:\s|$)/,
  /(?:^|\s)(?:sm|md|lg|xl|2xl):(?:inline|inline-flex|inline-block|block|flex|grid|contents)(?:\s.*)?\shidden(?:\s|$)/,
  /(?:^|\s)max-(?:sm|md|lg|xl|2xl):hidden(?:\s|$)/,
];
const hiddenOnPhone = (classes: string) => RESPONSIVE_HIDDEN.some((pattern) => pattern.test(classes));

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
/** Every class written in `className`: its strings and templates, wherever they sit (`cn(...)` too). */
const literalClass = (node: ts.JsxOpeningLikeElement) => {
  const parts: string[] = [];
  const collect = (value: ts.Node) => {
    if (ts.isStringLiteral(value) || ts.isNoSubstitutionTemplateLiteral(value)) parts.push(value.text);
    else if (ts.isTemplateExpression(value)) parts.push(value.head.text, ...value.templateSpans.map((span) => span.literal.text));
    ts.forEachChild(value, collect);
  };
  const value = attribute(node, "className")?.initializer;
  if (value) collect(value);
  return ` ${parts.join(" ")} `;
};

/** Whether `children` show text at every width: a text child, or an expression that is not an icon. */
function visibleText(children: ts.NodeArray<ts.JsxChild>): boolean {
  return children.some((child) => {
    if (ts.isJsxText(child)) return child.text.trim() !== "";
    if (ts.isJsxExpression(child)) return child.expression !== undefined && !ts.isJsxElement(child.expression) && !ts.isJsxSelfClosingElement(child.expression) && !ts.isConditionalExpression(child.expression);
    if (ts.isJsxElement(child)) return !hiddenOnPhone(literalClass(child.openingElement)) && visibleText(child.children);
    return false;
  });
}

const hidesText = (children: ts.NodeArray<ts.JsxChild>): boolean =>
  children.some((child) => ts.isJsxElement(child) && (hiddenOnPhone(literalClass(child.openingElement)) || hidesText(child.children)));

/** Controls in `text` (the file `file`) that are icon-only on a phone and have no `aria-label` or `title`. */
function unnamed(file: string, text = readFileSync(file, "utf8")): string[] {
  const source = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
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
  it.each([
    ["a plain class", `<Button><RefreshCw /><span className="hidden sm:inline">Refresh</span></Button>`],
    ["cn(...)", `<Button><RefreshCw /><span className={cn("text-xs", "hidden md:inline")}>Refresh</span></Button>`],
    ["a template", `<button><Icon /><span className={\`hidden \${size} lg:inline-flex\`}>Refresh</span></button>`],
    ["max-sm:hidden", `<a href="/x"><Icon /><span className="max-sm:hidden">Open</span></a>`],
    ["a nested span", `<Link to="/x"><Icon /><span><span className="hidden sm:inline">Open</span></span></Link>`],
  ])("finds a control hidden on a phone through %s", (_how, jsx) => {
    expect(unnamed("sample.tsx", `export const x = ${jsx};`)).toEqual(["sample.tsx:1 <" + jsx.slice(1, jsx.search(/[ >]/)) + ">"]);
  });

  it.each([
    ["an aria-label", `<Button aria-label="Refresh"><RefreshCw /><span className="hidden sm:inline">Refresh</span></Button>`],
    ["a title", `<AsyncActionButton title="Refresh"><span className="hidden sm:inline">Refresh</span></AsyncActionButton>`],
    ["text at every width", `<Button><Plus />New<span className="hidden sm:inline"> pool</span></Button>`],
  ])("accepts a control named by %s", (_how, jsx) => {
    expect(unnamed("sample.tsx", `export const x = ${jsx};`)).toEqual([]);
  });

  it("each carry an aria-label or title", () => {
    expect(components(join(ROOT, "src")).flatMap((file) => unnamed(file))).toEqual([]);
  });
});
