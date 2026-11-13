import { describe, expect, it } from "vitest";
import { CERT_EVENTS, DEFAULT_EVENT_TYPES, SYSTEM_EVENTS, eventLabel } from "./events";

describe("upgrade notification types", () => {
  it("keeps the six certificate subscriptions and adds both system subscriptions", () => {
    expect(CERT_EVENTS).toHaveLength(6);
    expect(SYSTEM_EVENTS.map((event) => event.type)).toEqual([
      "system.db_upgrade_backup_kept",
      "system.db_upgrade_activation_failed",
    ]);
    for (const event of SYSTEM_EVENTS) {
      expect(DEFAULT_EVENT_TYPES).toContain(event.type);
      expect(eventLabel(event.type)).toBe(`시스템 · ${event.label}`);
    }
    expect(eventLabel("unknown.type")).toBe("unknown.type");
  });
});
