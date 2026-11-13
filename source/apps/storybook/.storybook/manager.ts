import { addons } from "storybook/manager-api";
import { GLOBALS_UPDATED, SET_GLOBALS } from "storybook/internal/core-events";
import { normalizeTheme, toolThemes } from "./theme";

// The sidebar and toolbar follow the same `theme` global as the preview and Docs.
addons.setConfig({ theme: toolThemes.light });
addons.register("nwui/tool-theme", () => {
  const follow = ({ globals }: { globals?: Record<string, unknown> }) => {
    addons.setConfig({ theme: toolThemes[normalizeTheme(globals?.theme)] });
  };
  const channel = addons.getChannel();
  channel.on(SET_GLOBALS, follow);
  channel.on(GLOBALS_UPDATED, follow);
});
