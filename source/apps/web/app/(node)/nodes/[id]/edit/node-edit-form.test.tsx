import { expect, it, vi } from "vitest";
import {
  Children,
  isValidElement,
  type ComponentProps,
  type ReactElement,
} from "react";
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useActionState: () => [{ ok: false }, () => undefined, false],
  useState: (initial: unknown) => [initial, () => undefined],
}));
vi.mock("@/lib/use-api-action", () => ({
  useApiAction: () => () => undefined,
}));
import { NodeEditForm } from "./node-edit-form";
function render() {
  const element = NodeEditForm({
    id: "n1",
    defaults: {
      name: "Node",
      baseUrl: "https://node.example.test",
      allowInsecureTls: true,
    },
    tokenRole: "admin",
    registeredGatewayName: null,
  });
  const form = Children.toArray(element.props.children).find(
    (child) => isValidElement(child) && child.type === "form",
  );
  if (!isValidElement(form)) throw new Error("missing settings form");
  return form as ReactElement<ComponentProps<"form">>;
}
it("prevents the native reset and removes the handler when the ref is cleaned up", () => {
  const ref = render().props.ref;
  if (typeof ref !== "function") throw new Error("missing form callback ref");
  const form = { onreset: null } as HTMLFormElement;
  const cleanup = ref(form);
  const event = new Event("reset", { cancelable: true });
  form.onreset!.call(form, event);
  expect(event.defaultPrevented).toBe(true);
  expect(cleanup).toBeTypeOf("function");
  if (typeof cleanup === "function") cleanup();
  expect(form.onreset).toBeNull();
});
it("keeps the same callback across renders so action commits do not detach the handler", () => {
  expect(render().props.ref).toBe(render().props.ref);
});

import { renderToStaticMarkup } from "react-dom/server";
it("associates the one save button with the settings form without adding editable token fields", () => {
  const html = renderToStaticMarkup(
    <NodeEditForm
      id="n1"
      defaults={{
        name: "Node",
        baseUrl: "https://node.example.test",
        allowInsecureTls: true,
      }}
      tokenRole="admin"
      registeredGatewayName={null}
    />,
  );
  expect(html).toContain('id="node-edit-form"');
  expect(html).toContain('form="node-edit-form"');
  expect(html).toContain("<footer");
  expect(html).not.toContain('name="tokenRole"');
  expect(html).not.toContain('name="gatewayName"');
  expect(html).not.toContain('name="token"');
});
