import { useState } from "react";
import { NotificationBellView, type NotificationItem } from "@/components/notifications/notification-bell-view";
import { usePreviewNav } from "./preview-nav";

/** The real view, with local fixture state: no polling or seen POST in Storybook. */
export function PreviewNotifications({ items = [], unreadCount = 0 }: {
  items?: NotificationItem[];
  unreadCount?: number;
}) {
  const [unread, setUnread] = useState(unreadCount);
  const navigate = usePreviewNav();
  return <NotificationBellView items={items} unread={unread} onOpen={() => setUnread(0)} onNavigate={navigate} />;
}
