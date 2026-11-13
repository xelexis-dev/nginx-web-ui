import { expect, it } from "vitest";
import { assertValidDirective, parseFreeformDirectives, type DirectiveContext } from "./directive-validation";
import fixture from "./directive-refusals.fixture.json";

it.each(fixture.cases)("refuses $name in $context with the shared resolver hint", ({ name, context, argument }) => {
  const message = `Directive not allowed in ${context} context: "${name}" ${fixture.hint}`;
  expect(() => assertValidDirective(name, [argument], context as DirectiveContext)).toThrowError(new Error(message));
  expect(() => parseFreeformDirectives(`${name} ${argument};`, context as DirectiveContext)).toThrowError(new Error(message));
});
it("keeps unrelated refusal and allowed directive behavior", () => {
  expect(() => assertValidDirective("include", ["/tmp/config"], "server"))
    .toThrowError(new Error('Directive not allowed in server context: "include"'));
  expect(() => assertValidDirective("proxy_read_timeout", ["60s"], "location")).not.toThrow();
});
