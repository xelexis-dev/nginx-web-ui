import { Unstyled } from "@storybook/addon-docs/blocks";
import type { ReactNode } from "react";

// A live example inside an MDX page. Unstyled keeps Storybook's docs typography off the
// component, so it looks as it does in the app; the frame is drawn with the app's tokens.
export function DocExample({ children, label }: { children: ReactNode; label?: string }) {
  return (
    <Unstyled>
      <figure className="nwui-doc-example">
        {label ? <figcaption>{label}</figcaption> : null}
        <div className="nwui-doc-example-body">{children}</div>
      </figure>
    </Unstyled>
  );
}
