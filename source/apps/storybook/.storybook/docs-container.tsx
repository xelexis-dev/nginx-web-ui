import { applyTheme } from "@repo/ui";
import { DocsContainer, type DocsContainerProps } from "@storybook/addon-docs/blocks";
import { useEffect, useSyncExternalStore, type PropsWithChildren } from "react";
import { GLOBALS_UPDATED, SET_GLOBALS } from "storybook/internal/core-events";
import { normalizeTheme, toolThemes, type Theme } from "./theme";

// A standalone MDX page runs no story decorators, so Docs follows the toolbar itself: the same
// channel events the built-in blocks use, including the last payload for a page opened directly
// (a shared link, a reload, browser back). It sets the app's `data-theme` for token-styled content
// and the matching Storybook theme for prose, prop tables and code blocks.
export function ThemedDocsContainer({ context, children }: PropsWithChildren<DocsContainerProps>) {
  const channel = context.channel;
  const theme = useSyncExternalStore<Theme>(
    (notify) => {
      channel.on(GLOBALS_UPDATED, notify);
      channel.on(SET_GLOBALS, notify);
      return () => {
        channel.off(GLOBALS_UPDATED, notify);
        channel.off(SET_GLOBALS, notify);
      };
    },
    () =>
      normalizeTheme(
        (channel.last(GLOBALS_UPDATED)?.[0]?.globals ?? channel.last(SET_GLOBALS)?.[0]?.globals)?.theme,
      ),
    () => "light",
  );
  useEffect(() => applyTheme(theme), [theme]);
  return (
    <DocsContainer context={context} theme={toolThemes[theme]}>
      {children}
    </DocsContainer>
  );
}
