"use client";

import { startVisibleInterval } from "@/lib/visible-interval";
import { useCallback, useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { useToast } from "./toast-provider";
import { NotificationBellView, type NotificationItem } from "./notification-bell-view";

const POLL_MS = 30_000;

export function NotificationBell() {
  const [items, setItems] = useState<NotificationItem[]>([]);
  const [unread, setUnread] = useState(0);
  const seenIds = useRef<Set<string> | null>(null); // null=최초 로드 전(토스트 안 함)
  const router = useRouter();
  const { toast } = useToast();

  const load = useCallback(async () => {
    try {
      const res = await fetch("/api/ui/v1/notifications", { cache: "no-store" });
      if (!res.ok) return;
      const data: { items: NotificationItem[]; unreadCount: number } = await res.json();
      setItems(data.items);
      setUnread(data.unreadCount);
      if (seenIds.current === null) {
        seenIds.current = new Set(data.items.map((i) => i.id)); // 최초: 토스트 없이 기준선
      } else {
        // 오래된→새 순으로 새 항목만 토스트
        for (const it of [...data.items].reverse()) {
          if (!seenIds.current.has(it.id)) {
            seenIds.current.add(it.id);
            toast({ severity: it.severity, title: it.title, body: it.body, link: it.link });
          }
        }
      }
    } catch {
      // 폴 실패는 조용히 무시(다음 주기 재시도)
    }
  }, [toast]);

  useEffect(() => {
    void load();
    return startVisibleInterval(() => void load(), POLL_MS);
  }, [load]);

  async function markSeen() {
    // 열 때는 unread 값과 무관하게 항상 seen을 찍는다. 서버가 seenAt=now로
    // 갱신해 "지금까지의" 모든 알림을 커버하므로, 최초 GET이 아직 안 끝난
    // 상태에서 열어도(클라 unread=0) 뒤늦게 도착한 unread가 배지를 되살리지
    // 않는다(리뷰 A4). unread=0일 때의 여분 POST는 무해.
    setUnread(0);
    try {
      await fetch("/api/ui/v1/notifications/seen", { method: "POST" });
    } catch {
      /* 무시 */
    }
  }

  return <NotificationBellView items={items} unread={unread} onOpen={markSeen} onNavigate={router.push} />;
}
