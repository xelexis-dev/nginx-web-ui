import { Combobox, Field, FilterChips, Select } from "@repo/ui";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

// Shared form states (color-foundation-fixes): a Field's error must reach the real control of Select and
// Combobox the same way it reaches TextInput. How the states look (error border on hover/open, focus ring,
// disabled chips) is measured in the browser, not asserted from the CSS text here.

// The attributes of the first element matching `tag` whose markup contains `marker`.
function attrsOf(html: string, tag: string, marker: string) {
  const el = [...html.matchAll(new RegExp(`<${tag}\\b[^>]*>`, "g"))].map((m) => m[0]).find((m) => m.includes(marker));
  if (!el) throw new Error(`no <${tag}> with ${marker} in ${html}`);
  return Object.fromEntries([...el.matchAll(/([\w-]+)="([^"]*)"/g)].map((m) => [m[1], m[2]]));
}

describe("Field → Select / Combobox", () => {
  it("marks the Select trigger invalid and links its error", () => {
    const html = renderToStaticMarkup(
      <Field label="프로토콜" htmlFor="p" hint="TCP 또는 UDP" error="고르세요" required>
        <Select options={[{ value: "tcp", label: "TCP" }]} />
      </Field>,
    );
    const a = attrsOf(html, "button", 'id="p"');
    expect(a["aria-invalid"]).toBe("true");
    expect(a["aria-required"]).toBe("true");
    expect(a["aria-describedby"]?.split(" ")).toEqual(["p-hint", "p-error"]);
  });

  it("marks the Combobox input invalid and links its error", () => {
    const html = renderToStaticMarkup(
      <Field label="헤더" htmlFor="h" error="공백 불가" required>
        <Combobox options={[{ value: "Host" }]} value="X Y" onChange={() => undefined} />
      </Field>,
    );
    const a = attrsOf(html, "input", 'id="h"');
    expect(a["aria-invalid"]).toBe("true");
    expect(a["aria-required"]).toBe("true");
    expect(a["aria-describedby"]).toBe("h-error");
  });

  it("leaves a valid Combobox without error attributes", () => {
    const html = renderToStaticMarkup(
      <Field label="헤더" htmlFor="h2">
        <Combobox options={[]} value="" onChange={() => undefined} />
      </Field>,
    );
    const a = attrsOf(html, "input", 'id="h2"');
    expect(a["aria-invalid"]).toBeUndefined();
    expect(a["aria-describedby"]).toBeUndefined();
  });
});

describe("FilterChips disabled", () => {
  it("disables every chip and keeps the pressed one pressed", () => {
    const options = [
      { value: "all", label: "전체" },
      { value: "on", label: "활성" },
    ];
    const html = renderToStaticMarkup(<FilterChips label="필터" value="on" onChange={() => undefined} options={options} disabled />);
    const buttons = [...html.matchAll(/<button\b[^>]*>/g)].map((m) => m[0]);
    expect(buttons).toHaveLength(2);
    expect(buttons.every((b) => /\sdisabled=""/.test(b))).toBe(true);
    expect(buttons.filter((b) => b.includes('aria-pressed="true"'))).toHaveLength(1);
    const enabled = renderToStaticMarkup(<FilterChips label="필터" value="on" onChange={() => undefined} options={options} />);
    expect(enabled).not.toContain("disabled");
  });
});
